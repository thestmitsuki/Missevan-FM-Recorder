use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
// 仅供非测试构建使用（notifier 字段 / with_notifier，测试构建不链入 dispatcher 代码）
#[cfg(not(test))]
use std::sync::Arc;

use crate::domain::config::model::{
    is_valid_record_format, AnchorConfig, Config, GlobalConfig,
};
use crate::infrastructure::crypto;
use crate::infrastructure::error::types::AppError;
#[cfg(not(test))]
use crate::infrastructure::notification::dispatcher::NotificationDispatcher;

/// 备份文件前缀：`config.toml.bak.<YYYYMMDDHHMMSSmmm>`（固定宽度时间戳，字典序 = 时间序）
const BACKUP_PREFIX: &str = "config.toml.bak.";
/// 保留最近备份份数（规格 4.2：保留最近 5 个备份）
const MAX_BACKUPS: usize = 5;

/// 全局配置原子写锁（M1 审查跟进）：std Mutex 串行化「写临时文件 → rename 替换」
/// 整段序列。atomic_write_global 是同步函数（锁内无 await），因此用 std 而非
/// tokio Mutex。无锁时 tokio 多线程下两个并发写（save_config ∥ import_config /
/// set_shortcut，或 load() 损坏恢复回写 ∥ save）共享同一 `config.toml.tmp`，
/// 可交错出「A 写 tmp → B 覆盖 tmp → A remove+rename（B 内容落盘）→ B remove
/// 删掉 config.toml → B rename 失败」的竞态：最终 config.toml 缺失，且 load()
/// 对缺失文件静默返回默认配置——配置表现为静默丢失。
static GLOBAL_CONFIG_WRITE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
/// 临时文件序号（进程内单调递增；与 pid 组合成唯一 tmp 文件名，见
/// atomic_write_global——即便未来某路径漏拿锁，写者也不会共用同一 tmp 互相覆盖）
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// 配置管理器——负责加载、保存、迁移配置（Task 18：写盘前备份 + 敏感字段混淆 + 损坏自动恢复）
pub struct ConfigManager {
    data_dir: PathBuf,
    /// 通知分发器（备份恢复通知、通知设置同步）；测试构建不编译（见 with_notifier 注释）
    #[cfg(not(test))]
    notifier: Option<Arc<NotificationDispatcher>>,
}

impl ConfigManager {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            #[cfg(not(test))]
            notifier: None,
        }
    }

    /// 注入通知分发器（配置备份恢复通知、通知设置同步；Task 18）。
    /// `cfg(not(test))`：Option<Arc<NotificationDispatcher>> 字段的 drop glue 会把
    /// tauri 运行时代码链入测试二进制（本机 rust-lld + Windows 下测试可执行文件
    /// 无法加载 0xC0000139，见 dispatcher.rs 测试注释）；生产构建不受影响。
    #[cfg(not(test))]
    pub fn with_notifier(mut self, notifier: Arc<NotificationDispatcher>) -> Self {
        self.notifier = Some(notifier);
        self
    }

    /// 配置目录路径（平台相关）
    fn config_dir(&self) -> PathBuf {
        self.data_dir.clone()
    }

    /// 全局配置文件路径
    pub fn global_config_path(&self) -> PathBuf {
        self.config_dir().join("config.toml")
    }

    /// 主播配置目录
    fn anchors_dir(&self) -> PathBuf {
        self.config_dir().join("anchors")
    }

    /// 单主播配置文件路径（H2：id 经白名单校验后才拼入路径——拒绝空、路径
    /// 分隔符、`..` 等穿越成分，杜绝任意 .toml 路径写入）
    fn anchor_path(&self, id: &str) -> Result<PathBuf, AppError> {
        validate_anchor_id(id)?;
        Ok(self.anchors_dir().join(format!("{}.toml", id)))
    }

    /// 加载完整配置（全局 + 所有主播）。
    ///
    /// - 全局配置解析失败（损坏）时自动按新→旧回溯备份（规格 4.2，保留策略内最多
    ///   5 份，取首个可解析者），恢复成功后把备份内容写回 config.toml 修复磁盘文件，
    ///   并通知用户（`config_recovered`）；全部备份不可解析则返回原始错误
    /// - 敏感字段（proxy_password / cookie）从磁盘解密；
    ///   旧版明文配置由 `deobfuscate_or_plain` 回退原样返回（读兼容，不破坏）
    pub fn load(&self) -> Result<Config, AppError> {
        let global_path = self.global_config_path();
        if !global_path.exists() {
            return Ok(Config {
                global: GlobalConfig::default(),
                anchors: Vec::new(),
            });
        }

        let global_str = std::fs::read_to_string(&global_path).map_err(|e| {
            AppError::config(format!("读取配置失败: {}", e)).with_source("config_manager")
        })?;

        let mut global: GlobalConfig = match toml::from_str(&global_str) {
            Ok(g) => g,
            Err(parse_err) => {
                // 损坏：按新→旧遍历备份（保留策略内最多 5 份），取首个可解析者恢复。
                // 最新备份本身也可能损坏/半写——回溯更旧备份避免恢复失败；
                // 全部不可解析则返回原始解析错误。
                let mut backups = self.list_backups();
                backups.sort(); // 固定宽度时间戳文件名：字典序 = 时间序（旧→新）
                let mut recovered: Option<(PathBuf, GlobalConfig)> = None;
                for backup in backups.iter().rev() {
                    let Ok(s) = std::fs::read_to_string(backup) else {
                        continue;
                    };
                    if let Ok(g) = toml::from_str::<GlobalConfig>(&s) {
                        // 恢复成功后把备份内容写回 config.toml：修复磁盘上的损坏文件，
                        // 避免下次启动/下次命令再次走恢复路径（写回失败不阻断本次加载）；
                        // 与 save_global 一致用临时文件 + rename 原子替换（M1）
                        if let Err(e) = self.atomic_write_global(&s) {
                            tracing::warn!("配置恢复后写回 config.toml 失败（本次加载不受影响）: {}", e);
                        }
                        recovered = Some((backup.clone(), g));
                        break;
                    }
                }
                let Some((backup, recovered)) = recovered else {
                    return Err(parse_error(&parse_err));
                };
                let backup_name = backup
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                tracing::warn!(
                    "配置文件损坏（{}），已从备份恢复: {}",
                    parse_err,
                    backup_name
                );
                #[cfg(not(test))]
                self.notify_recovered(&backup_name);
                recovered
            }
        };

        // 解密敏感字段（旧明文配置按原样返回）
        let key = crypto::machine_key();
        global.proxy_password = crypto::deobfuscate_or_plain(&global.proxy_password, &key);

        let mut anchors = Vec::new();
        let anchors_dir = self.anchors_dir();
        if anchors_dir.exists() {
            for entry in std::fs::read_dir(&anchors_dir)
                .map_err(|e| AppError::config(format!("读取主播目录失败: {}", e)))?
            {
                let entry = entry.map_err(|_| AppError::internal("读取目录条目失败"))?;
                if entry.path().extension().map_or(false, |ext| ext == "toml") {
                    let anchor_str = std::fs::read_to_string(entry.path())
                        .map_err(|e| AppError::config(format!("读取主播配置失败: {}", e)))?;
                    if let Ok(mut anchor) = toml::from_str::<AnchorConfig>(&anchor_str) {
                        if let Some(cookie) = &anchor.cookie {
                            anchor.cookie = Some(crypto::deobfuscate_or_plain(cookie, &key));
                        }
                        anchors.push(anchor);
                    }
                }
            }
        }

        Ok(Config { global, anchors })
    }

    /// 备份恢复通知（应用内 + tracing 日志；不触发系统通知——不在事件勾选矩阵内）。
    /// 异步 fire-and-forget：load() 被几乎所有 async 命令（get_config / save_config /
    /// get_anchors 等）调用，运行在 tokio runtime worker 线程上——从该上下文
    /// `block_on` 会触发 `try_enter_blocking_region()` 失败 panic（"Cannot block the
    /// current thread from within a runtime"）。改为 `spawn` 后台任务发送通知，
    /// 不阻塞配置加载路径；通知发送失败仅记录日志，不影响加载结果。
    /// `cfg(not(test))`：测试构建不链入 NotificationDispatcher 的 tauri 事件代码
    /// （本机 rust-lld + Windows 下会致测试可执行文件无法加载，见 dispatcher.rs 测试注释）
    #[cfg(not(test))]
    fn notify_recovered(&self, backup_name: &str) {
        let Some(notifier) = &self.notifier else {
            return;
        };
        let notifier = notifier.clone();
        let backup_name = backup_name.to_string();
        tauri::async_runtime::spawn(async move {
            notifier
                .warning(
                    "config_recovered",
                    "配置已从备份恢复",
                    format!(
                        "配置文件损坏，已自动恢复备份（{}）。请检查设置并重新保存。",
                        backup_name
                    ),
                )
                .await;
        });
    }

    /// 保存全局配置（Task 18）：
    /// 1. 写盘前备份旧文件（`config.toml.bak.<时间戳>`，保留最近 5 份）
    /// 2. proxy_password 混淆后落盘（读取时解密；旧明文配置读兼容）
    /// 3. 成功后同步通知设置到分发器（系统通知开关 / 事件勾选即时生效）
    pub fn save_global(&self, config: &GlobalConfig) -> Result<(), AppError> {
        // M1 审查跟进：record_format 会被拼入输出文件扩展名（engine.rs 输出路径），
        // 白名单校验拒绝 `../../` 等路径穿越注入。save_config / import_config /
        // set_shortcut / set_autostart 全部写路径都经本函数落盘，一处把关全盖。
        if !is_valid_record_format(&config.record_format) {
            return Err(AppError::config(format!(
                "不支持的录制格式: {}（仅支持 m4a / mp3）",
                config.record_format
            )));
        }
        let dir = self.config_dir();
        std::fs::create_dir_all(&dir).map_err(|e| {
            AppError::system(
                crate::infrastructure::error::types::IO_WRITE_FAIL,
                "创建配置目录失败",
            )
            .with_technical(format!("{}", e))
        })?;

        // 1. 备份旧文件（不存在则跳过；备份失败不阻断保存——正常读写不受影响）
        if let Err(e) = self.backup_global() {
            tracing::warn!("创建配置备份失败（已跳过）: {}", e);
        }

        // 2. 敏感字段混淆后序列化
        let mut cfg = config.clone();
        cfg.proxy_password = crypto::obfuscate(&config.proxy_password, &crypto::machine_key());
        let toml_str = toml::to_string_pretty(&cfg)
            .map_err(|e| AppError::config(format!("序列化配置失败: {}", e)))?;

        // 3. 原子写（M1）：临时文件 + rename 替换，避免「截断写入窗口被并发
        // load() 读到半写文件 → 走备份恢复 → 回写覆盖新配置」的竞态。
        self.atomic_write_global(&toml_str)?;

        // 3. 同步通知设置（save_config / import / set_shortcut 等全部写路径即时生效）
        #[cfg(not(test))]
        if let Some(notifier) = &self.notifier {
            notifier.sync_from_config(config);
        }

        Ok(())
    }

    /// 原子写全局配置（M1 + 审查跟进）：
    /// 1. 全程持 `GLOBAL_CONFIG_WRITE_LOCK`——锁粒度覆盖「写临时文件 → rename
    ///    替换」整段序列，并发写命令（save_config / import_config / set_shortcut
    ///    / set_autostart / load 恢复回写）串行化，杜绝共享 tmp 互相覆盖与
    ///    remove/rename 交错（曾可致 config.toml 被删后 rename 失败 → 文件缺失）
    /// 2. 临时文件名唯一化（`config.toml.tmp.<pid>.<seq>`）作双保险：即使未来
    ///    某调用路径漏拿锁，两个写者也各写各的 tmp，不会互相覆盖
    /// 3. 直接 rename 替换目标，不先 remove：Windows 下 std::fs::rename 走
    ///    MoveFileExW 且带 MOVEFILE_REPLACE_EXISTING，Unix 下 rename(2) 同样
    ///    原子替换——两种平台 rename 都覆盖已存在目标；先 remove 反而制造
    ///    config.toml 短暂缺失的窗口，并发 load() 会静默读到默认配置。
    ///    临时文件残留（异常中断）不影响下次加载——load 只读 `config.toml`。
    fn atomic_write_global(&self, content: &str) -> Result<(), AppError> {
        let target = self.global_config_path();
        let tmp = self.config_dir().join(format!(
            "config.toml.tmp.{}.{}",
            std::process::id(),
            TMP_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        // 中毒容忍：锁内只做 IO（无 panic 面），真被毒化时继续执行不阻塞后续写
        let _guard = GLOBAL_CONFIG_WRITE_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        std::fs::write(&tmp, content).map_err(|e| {
            AppError::system(
                crate::infrastructure::error::types::IO_WRITE_FAIL,
                "写入配置失败",
            )
            .with_technical(format!("{}", e))
        })?;
        std::fs::rename(&tmp, &target).map_err(|e| {
            AppError::system(
                crate::infrastructure::error::types::IO_WRITE_FAIL,
                "写入配置失败",
            )
            .with_technical(format!("{}", e))
        })
    }

    /// 添加主播（cookie 混淆后落盘；读取时解密；旧明文配置读兼容）
    pub fn add_anchor(&self, anchor: &AnchorConfig) -> Result<(), AppError> {
        let dir = self.anchors_dir();
        std::fs::create_dir_all(&dir).map_err(|e| {
            AppError::system(
                crate::infrastructure::error::types::IO_WRITE_FAIL,
                "创建主播目录失败",
            )
            .with_technical(format!("{}", e))
        })?;

        let mut stored = anchor.clone();
        stored.cookie = anchor
            .cookie
            .as_deref()
            .map(|c| crypto::obfuscate(c, &crypto::machine_key()));
        let toml_str = toml::to_string_pretty(&stored)
            .map_err(|e| AppError::config(format!("序列化主播配置失败: {}", e)))?;

        std::fs::write(self.anchor_path(&anchor.id)?, toml_str).map_err(|e| {
            AppError::system(
                crate::infrastructure::error::types::IO_WRITE_FAIL,
                "写入主播配置失败",
            )
            .with_technical(format!("{}", e))
        })?;

        Ok(())
    }

    // ── 备份（规格 4.2：保存前备份，保留最近 5 份；损坏自动恢复） ──

    /// 备份当前 config.toml 到 `config.toml.bak.<时间戳>`，并裁剪到最近 MAX_BACKUPS 份。
    /// 文件不存在（首次保存）时跳过。
    fn backup_global(&self) -> Result<(), AppError> {
        let path = self.global_config_path();
        if !path.exists() {
            return Ok(());
        }
        let content = std::fs::read(&path).map_err(|e| {
            AppError::system(
                crate::infrastructure::error::types::IO_WRITE_FAIL,
                "读取旧配置（备份源）失败",
            )
            .with_technical(format!("{}", e))
        })?;
        // 微秒时间戳 + 进程内序号后缀：同一微秒内多次保存也不覆盖（回归：同毫秒
        // 保存会互相覆盖备份）。序号补零保持固定宽度——文件名字典序 = 时间序。
        static BACKUP_SEQ: AtomicU64 = AtomicU64::new(0);
        let ts = chrono::Local::now().format("%Y%m%d%H%M%S%.6f");
        let seq = BACKUP_SEQ.fetch_add(1, Ordering::Relaxed) % 100;
        let backup = self
            .config_dir()
            .join(format!("{}{}.{:02}", BACKUP_PREFIX, ts, seq));
        std::fs::write(&backup, &content).map_err(|e| {
            AppError::system(
                crate::infrastructure::error::types::IO_WRITE_FAIL,
                "写入配置备份失败",
            )
            .with_technical(format!("{}", e))
        })?;
        self.prune_backups();
        Ok(())
    }

    /// 保留最近 MAX_BACKUPS 份备份，删除更旧的（固定宽度时间戳文件名，字典序 = 时间序）
    fn prune_backups(&self) {
        let mut backups = self.list_backups();
        if backups.len() <= MAX_BACKUPS {
            return;
        }
        backups.sort();
        let remove_count = backups.len() - MAX_BACKUPS;
        for p in backups.into_iter().take(remove_count) {
            let _ = std::fs::remove_file(p);
        }
    }

    /// 全部备份文件（未排序）
    fn list_backups(&self) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(self.config_dir()) else {
            return Vec::new();
        };
        entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.is_file()
                    && p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with(BACKUP_PREFIX))
            })
            .collect()
    }

    /// 删除主播
    pub fn remove_anchor(&self, id: &str) -> Result<(), AppError> {
        let path = self.anchor_path(id)?;
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| {
                AppError::system(
                    crate::infrastructure::error::types::IO_WRITE_FAIL,
                    "删除主播配置失败",
                )
                .with_technical(format!("{}", e))
            })?;
        }
        Ok(())
    }

    /// 判断是否首次运行（无配置文件，或配置存在但引导未完成）。
    ///
    /// 引导完成标记：首次向导第 3 步检查通过即写盘（config.toml 存在），但
    /// 用户未到第 4 步「进入应用」就退出时引导未完成——此时必须再次打开引导窗
    /// （规格「若用户未完成配置而关闭，再次启动时仍会重新打开引导窗口」）。
    pub fn is_first_run(&self) -> bool {
        if !self.global_config_path().exists() {
            return true;
        }
        // 配置存在但引导未完成（wizard_completed=false，首次写盘时显式设置）
        match self.load() {
            Ok(config) => !config.global.wizard_completed,
            Err(_) => true, // 配置损坏：保守视为首次运行（引导可重建）
        }
    }

    /// 删除全部配置（config.toml + anchors/），供 reset_config 使用。
    /// 仅当配置目录存在时才删除；不存在视为成功（幂等）。
    pub fn delete_all(&self) -> Result<(), AppError> {
        let dir = self.config_dir();
        if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(|e| {
                AppError::system(
                    crate::infrastructure::error::types::IO_WRITE_FAIL,
                    "删除配置目录失败",
                )
                .with_technical(format!("{}", e))
            })?;
        }
        Ok(())
    }

    /// 删除全部主播配置（import replace 全替换时先清空再写入）
    pub fn remove_all_anchors(&self) -> Result<(), AppError> {
        let dir = self.anchors_dir();
        if !dir.exists() {
            return Ok(());
        }
        for entry in std::fs::read_dir(&dir)
            .map_err(|e| AppError::config(format!("读取主播目录失败: {}", e)))?
        {
            let entry = entry.map_err(|_| AppError::internal("读取目录条目失败"))?;
            if entry.path().extension().map_or(false, |ext| ext == "toml") {
                std::fs::remove_file(entry.path()).map_err(|e| {
                    AppError::system(
                        crate::infrastructure::error::types::IO_WRITE_FAIL,
                        "删除主播配置失败",
                    )
                    .with_technical(format!("{}", e))
                })?;
            }
        }
        Ok(())
    }

    /// 导出配置为 JSON 字符串（§11.2 export_config）：
    /// `{ "version": 1, "global": {...}, "anchors": [...] }`，
    /// 敏感字段置空/脱敏（global.proxy_password、anchor.cookie；代理 URL 内嵌
    /// 密码 → `:***@`，M3——与诊断报告导出 redact_config 规则一致）。
    pub fn export_json(&self) -> Result<String, AppError> {
        let config = self.load()?;
        let mut global = config.global.clone();
        global.proxy_password = String::new();
        global.proxy_addr = redact_proxy_url(&global.proxy_addr);
        let anchors: Vec<AnchorConfig> = config
            .anchors
            .into_iter()
            .map(|mut a| {
                a.cookie = None;
                if let Some(p) = a.proxy.as_deref() {
                    a.proxy = Some(redact_proxy_url(p));
                }
                a
            })
            .collect();
        let payload = serde_json::json!({
            "version": 1,
            "global": global,
            "anchors": anchors,
        });
        serde_json::to_string_pretty(&payload)
            .map_err(|e| AppError::config(format!("序列化导出配置失败: {}", e)))
    }

    /// 导入配置（§11.2 import_config）。
    ///
    /// 接受两种 JSON 形态：
    /// - 包裹式（本应用 export_config 的输出）：`{"global": {...}, "anchors": [...]}`
    /// - 扁平式（GlobalConfig 单对象；前端设置页当前导出格式）：整体视为 global，anchors 不涉及
    ///
    /// 模式：
    /// - `replace`：global 全替换；文件含 anchors 时主播列表全替换（删除本地多余主播）
    /// - `merge`：global 按字段合并（文件字段覆盖本地）；主播按 id 合并，重复 id 跳过（保留本地）
    pub fn import_json(&self, json: &str, mode: &str) -> Result<ImportSummary, AppError> {
        match mode {
            "replace" => self.import_replace(json),
            "merge" => self.import_merge(json),
            other => Err(AppError::config(format!(
                "不支持的导入模式: {}（仅支持 replace / merge）",
                other
            ))),
        }
    }

    /// replace 模式实现
    fn import_replace(&self, json: &str) -> Result<ImportSummary, AppError> {
        let value: serde_json::Value = serde_json::from_str(json)
            .map_err(|e| AppError::config(format!("导入文件不是有效 JSON: {}", e)))?;
        let wrapped = value.get("global").is_some();
        let global_value = if wrapped {
            value.get("global").cloned().unwrap_or(serde_json::Value::Null)
        } else {
            value.clone()
        };
        if !global_value.is_object() {
            return Err(AppError::config("导入的全局配置必须是 JSON 对象"));
        }
        let global: GlobalConfig = serde_json::from_value(global_value)
            .map_err(|e| AppError::config(format!("全局配置字段格式无效: {}", e)))?;

        // 主播列表：包裹式且含 anchors 时全替换；扁平式不动本地主播
        let mut anchors_removed = 0usize;
        let mut anchors_added = 0usize;
        let mut anchors_skipped = 0usize;
        let file_anchors: Option<Vec<AnchorConfig>> = if wrapped {
            let raw = value.get("anchors").cloned().unwrap_or(serde_json::Value::Null);
            if raw.is_null() {
                None
            } else {
                let parsed: Vec<AnchorConfig> = serde_json::from_value(raw)
                    .map_err(|e| AppError::config(format!("主播列表格式无效: {}", e)))?;
                Some(parsed)
            }
        } else {
            None
        };

        // 结构校验（写入前）：id 非空且文件内不重复（重复只取首个）
        let local_anchors = self.load()?.anchors;
        let mut seen = std::collections::HashSet::new();
        let mut deduped: Vec<&AnchorConfig> = Vec::new();
        if let Some(ref list) = file_anchors {
            for a in list {
                if let Err(e) = validate_anchor_id(&a.id) {
                    return Err(AppError::config(format!(
                        "主播列表包含非法 id，导入已中止: {}",
                        e.message
                    )));
                }
                if seen.insert(a.id.clone()) {
                    deduped.push(a);
                } else {
                    anchors_skipped += 1; // 文件内重复 id：保留首个
                }
            }
        }

        // 写入
        self.save_global(&global)?;
        if let Some(ref list) = file_anchors {
            // 全替换：先清空本地主播
            for a in &local_anchors {
                if !list.iter().any(|fa| fa.id == a.id) {
                    anchors_removed += 1;
                }
            }
            self.remove_all_anchors()?;
            for a in deduped {
                self.add_anchor(a)?;
                anchors_added += 1;
            }
        }

        Ok(ImportSummary {
            mode: "replace".to_string(),
            global_replaced: true,
            anchors_added,
            anchors_removed,
            anchors_skipped,
            anchors_total: file_anchors
                .as_ref()
                .map_or(local_anchors.len(), |l| l.len()),
        })
    }

    /// merge 模式实现
    fn import_merge(&self, json: &str) -> Result<ImportSummary, AppError> {
        let value: serde_json::Value = serde_json::from_str(json)
            .map_err(|e| AppError::config(format!("导入文件不是有效 JSON: {}", e)))?;
        let wrapped = value.get("global").is_some();
        let patch = if wrapped {
            value.get("global").cloned().unwrap_or(serde_json::Value::Null)
        } else {
            value.clone()
        };
        if !patch.is_object() {
            return Err(AppError::config("导入的全局配置必须是 JSON 对象"));
        }

        let current = self.load()?;
        // 字段级合并：本地 global 序列化为对象，文件字段覆盖，再整体反序列化
        let mut merged = serde_json::to_value(&current.global)
            .map_err(|e| AppError::config(format!("序列化本地配置失败: {}", e)))?;
        overlay_json(&mut merged, &patch);
        let global: GlobalConfig = serde_json::from_value(merged)
            .map_err(|e| AppError::config(format!("全局配置字段格式无效: {}", e)))?;

        // 主播解析 + 结构校验（全部前置，通过后才写盘）：
        // 列表解析失败或含空 id → 中止且不写入；文件内重复 id 只取首个（计 skipped）
        // 去重与 replace 路径一致：seen.insert 成功才入列，后续写入只遍历去重列表
        let mut file_anchors: Vec<AnchorConfig> = Vec::new();
        let mut skipped = 0usize;
        if wrapped {
            if let Some(raw) = value.get("anchors").cloned() {
                if !raw.is_null() {
                    let parsed: Vec<AnchorConfig> = serde_json::from_value(raw)
                        .map_err(|e| AppError::config(format!("主播列表格式无效: {}", e)))?;
                    let mut seen = std::collections::HashSet::new();
                    for a in parsed {
                        if let Err(e) = validate_anchor_id(&a.id) {
                            return Err(AppError::config(format!(
                                "主播列表包含非法 id，导入已中止: {}",
                                e.message
                            )));
                        }
                        if seen.insert(a.id.clone()) {
                            file_anchors.push(a);
                        } else {
                            skipped += 1; // 文件内重复 id：只取首个
                        }
                    }
                }
            }
        }

        // 校验全部通过：先写 global，再写新增主播（与本地重复者跳过，保留本地）
        self.save_global(&global)?;
        let mut added = 0usize;
        for a in &file_anchors {
            if current.anchors.iter().any(|la| la.id == a.id) {
                skipped += 1; // 本地已存在：保留本地
                continue;
            }
            self.add_anchor(a)?;
            added += 1;
        }

        Ok(ImportSummary {
            mode: "merge".to_string(),
            global_replaced: false,
            anchors_added: added,
            anchors_removed: 0,
            anchors_skipped: skipped,
            anchors_total: current.anchors.len() + added,
        })
    }
}

/// 代理 URL 内嵌密码脱敏（M3）：`http://user:pass@host` → `http://user:***@host`；
/// 无密码/解析失败原样返回。与诊断报告导出（debug_cmds::redact_config）共用，
/// 保证 config 导出与诊断导出脱敏规则一致。
pub fn redact_proxy_url(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(mut u) if u.password().is_some() => {
            let _ = u.set_password(Some("***"));
            u.to_string()
        }
        _ => url.to_string(),
    }
}

/// 主播 id 白名单校验（H2）：id 会被拼入 `anchors/{id}.toml` 路径，仅允许
/// `[A-Za-z0-9_-]`（1-64 字符）——UUID（含 `-`）与现有泛化 id（如 "a1"）均
/// 兼容，路径分隔符、`..`、空白等其他字符一律拒绝。
pub fn validate_anchor_id(id: &str) -> Result<(), AppError> {
    if id.is_empty() {
        return Err(AppError::config("主播 id 不能为空"));
    }
    if id.len() > 64 {
        return Err(AppError::config("主播 id 长度超限（最大 64 字符）"));
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(AppError::config(
            "主播 id 含非法字符（仅允许字母、数字、-、_）",
        ));
    }
    Ok(())
}

/// 构造 TOML 解析错误（load 损坏 / 备份也损坏时返回）
fn parse_error(e: &toml::de::Error) -> AppError {
    AppError::config(format!("解析配置失败: {}", e))
        .with_technical(format!("TOML 解析错误: {}", e))
        .with_source("config_manager")
}

/// 将 patch 对象的字段浅层覆盖到 base 对象上（用于 merge 模式字段级合并）
fn overlay_json(base: &mut serde_json::Value, patch: &serde_json::Value) {
    if let (Some(base_obj), Some(patch_obj)) = (base.as_object_mut(), patch.as_object()) {
        for (k, v) in patch_obj {
            base_obj.insert(k.clone(), v.clone());
        }
    }
}

/// 导入结果摘要（import_config 返回给前端）
#[derive(Debug, Clone, serde::Serialize)]
pub struct ImportSummary {
    pub mode: String,
    /// replace 模式下 global 是否已全替换
    pub global_replaced: bool,
    pub anchors_added: usize,
    pub anchors_removed: usize,
    pub anchors_skipped: usize,
    pub anchors_total: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::config::model::GlobalConfig;

    /// 唯一临时目录（并行测试隔离）
    fn unique_dir(tag: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        std::env::temp_dir().join(format!(
            "missevan-test-{}-{}-{}",
            tag,
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ))
    }

    fn list_backups(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut v: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
            .map(|it| {
                it.filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| {
                        p.file_name()
                            .and_then(|n| n.to_str())
                            .is_some_and(|n| n.starts_with("config.toml.bak."))
                    })
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    }

    // ── Task 18：备份 / 恢复 / 加密 ──
    // 注意：本模块测试不构造 NotificationDispatcher（其 AppHandle 字段会把
    // tauri 运行时代码链入测试二进制，本机 rust-lld + Windows 组合下测试
    // 可执行文件无法加载 0xC0000139——见 dispatcher.rs 测试注释）。

    #[test]
    fn save_creates_backups_and_prunes_to_five() {
        let dir = unique_dir("backup");
        let manager = ConfigManager::new(dir.clone());
        for i in 0..7u32 {
            let mut cfg = GlobalConfig::default();
            cfg.output_dir = format!("D:/rec-{}", i);
            manager.save_global(&cfg).unwrap();
        }
        // 6 次备份（第 0 次无旧文件不备份）→ 裁剪保留最近 5 份
        let backups = list_backups(&dir);
        assert_eq!(backups.len(), 5, "备份应保留最近 5 份: {:?}", backups);
        // 最近一份备份 = 第 6 次保存覆盖前的文件（内容为第 5 次保存）
        let latest = backups.last().unwrap();
        let parsed: GlobalConfig = toml::from_str(&std::fs::read_to_string(latest).unwrap()).unwrap();
        assert_eq!(parsed.output_dir, "D:/rec-5");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_recovers_from_latest_backup_when_corrupted() {
        let dir = unique_dir("recover");
        let manager = ConfigManager::new(dir.clone());

        let mut cfg = GlobalConfig::default();
        cfg.output_dir = "D:/recordings".to_string();
        cfg.proxy_password = "pw-secret".to_string();
        manager.save_global(&cfg).unwrap();
        cfg.output_dir = "E:/recordings".to_string();
        manager.save_global(&cfg).unwrap();

        // 损坏当前文件（绕过备份直接写入垃圾）
        std::fs::write(manager.global_config_path(), "{{{ not valid toml").unwrap();
        let loaded = manager.load().unwrap();
        // 最近备份 = 最后一次保存前的完整状态（第二次保存的备份源），恢复它
        assert_eq!(loaded.global.output_dir, "D:/recordings", "应从最近备份恢复");
        assert_eq!(loaded.global.proxy_password, "pw-secret", "恢复后敏感字段解密一致");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_recovers_from_older_backup_when_latest_is_corrupted() {
        let dir = unique_dir("recover-older");
        let manager = ConfigManager::new(dir.clone());

        let mut cfg = GlobalConfig::default();
        cfg.output_dir = "A:/one".to_string();
        manager.save_global(&cfg).unwrap(); // 无旧文件，不备份
        cfg.output_dir = "B:/two".to_string();
        manager.save_global(&cfg).unwrap(); // 备份 = A
        cfg.output_dir = "C:/three".to_string();
        manager.save_global(&cfg).unwrap(); // 备份 = B

        // 损坏当前文件 + 最新备份（内容为 B）→ 应回溯更旧备份（内容为 A）
        std::fs::write(manager.global_config_path(), "{{{ not valid toml").unwrap();
        let mut backups = list_backups(&dir);
        backups.sort();
        std::fs::write(backups.last().unwrap(), "{{{ latest backup also bad").unwrap();

        let loaded = manager.load().unwrap();
        assert_eq!(
            loaded.global.output_dir, "A:/one",
            "最新备份损坏时应回溯更旧备份"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_rewrites_recovered_config_back_to_disk() {
        let dir = unique_dir("recover-rewrite");
        let manager = ConfigManager::new(dir.clone());

        let mut cfg = GlobalConfig::default();
        cfg.output_dir = "D:/recordings".to_string();
        manager.save_global(&cfg).unwrap();
        cfg.output_dir = "E:/recordings".to_string();
        manager.save_global(&cfg).unwrap(); // 备份 = D:/recordings

        // 损坏当前文件 → 恢复后磁盘文件应被修复为可解析的备份内容
        std::fs::write(manager.global_config_path(), "{{{ not valid toml").unwrap();
        let loaded = manager.load().unwrap();
        assert_eq!(loaded.global.output_dir, "D:/recordings");

        let repaired: GlobalConfig =
            toml::from_str(&std::fs::read_to_string(manager.global_config_path()).unwrap())
                .unwrap();
        assert_eq!(
            repaired.output_dir, "D:/recordings",
            "恢复成功后应把备份内容写回 config.toml 修复磁盘文件"
        );
        // 修复后的文件再次加载不再走恢复路径
        assert_eq!(manager.load().unwrap().global.output_dir, "D:/recordings");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_returns_error_when_no_valid_backup() {
        let dir = unique_dir("nobackup");
        let manager = ConfigManager::new(dir.clone());
        // 从未保存过：只有损坏文件、无备份
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(manager.global_config_path(), "{{{ not valid toml").unwrap();
        assert!(manager.load().is_err());
        // 备份也损坏：同样报错
        std::fs::write(dir.join("config.toml.bak.20260801120000123"), "{{{ also bad").unwrap();
        assert!(manager.load().is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sensitive_fields_obfuscated_on_disk_and_restored_on_load() {
        let (manager, dir) = setup_config(); // proxy_password="secret"、a1.cookie="ck-a"
        let global_raw = std::fs::read_to_string(manager.global_config_path()).unwrap();
        assert!(
            !global_raw.contains("secret"),
            "磁盘上不得出现明文密码: {}",
            global_raw
        );
        assert!(global_raw.contains("enc:v1:"));
        let anchor_raw = std::fs::read_to_string(manager.anchors_dir().join("a1.toml")).unwrap();
        assert!(!anchor_raw.contains("ck-a"), "磁盘上不得出现明文 Cookie: {}", anchor_raw);
        assert!(anchor_raw.contains("enc:v1:"));

        let loaded = manager.load().unwrap();
        assert_eq!(loaded.global.proxy_password, "secret");
        let a1 = loaded.anchors.iter().find(|a| a.id == "a1").unwrap();
        assert_eq!(a1.cookie.as_deref(), Some("ck-a"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_plaintext_sensitive_fields_still_load() {
        let dir = unique_dir("legacy");
        let manager = ConfigManager::new(dir.clone());
        std::fs::create_dir_all(manager.anchors_dir()).unwrap();
        // 旧版：明文落盘
        std::fs::write(
            manager.global_config_path(),
            "output_dir = \"./recordings\"\nproxy_password = \"legacy-pw\"\n",
        )
        .unwrap();
        std::fs::write(
            manager.anchors_dir().join("a1.toml"),
            "id = \"a1\"\nname = \"主播A\"\nurl = \"https://m.missevan.com/live/1\"\nroom_id = \"1\"\nenable_check = true\ncookie = \"legacy-ck\"\n",
        )
        .unwrap();

        // 读兼容：明文原样返回
        let loaded = manager.load().unwrap();
        assert_eq!(loaded.global.proxy_password, "legacy-pw");
        assert_eq!(loaded.anchors[0].cookie.as_deref(), Some("legacy-ck"));

        // 再次保存后磁盘变为混淆格式，读取一致
        manager.save_global(&loaded.global).unwrap();
        let raw = std::fs::read_to_string(manager.global_config_path()).unwrap();
        assert!(!raw.contains("legacy-pw"));
        assert!(raw.contains("enc:v1:"));
        let reloaded = manager.load().unwrap();
        assert_eq!(reloaded.global.proxy_password, "legacy-pw");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn anchor_tags_persist_across_save_load() {
        // Task A/3：tags 落盘持久化——保存后加载一致，且磁盘 toml 含 tags
        let dir = unique_dir("anchor-tags");
        let manager = ConfigManager::new(dir.clone());
        // load() 依赖全局配置存在（否则视作首次运行返回空主播列表）
        manager.save_global(&GlobalConfig::default()).unwrap();
        let mut anchor = AnchorConfig {
            id: "t1".to_string(),
            name: "主播T".to_string(),
            url: "https://m.missevan.com/live/9".to_string(),
            room_id: "9".to_string(),
            proxy: None,
            cookie: None,
            enable_check: true,
            avatar_url: None,
            tags: vec!["日常".to_string(), "ASMR".to_string()],
        };
        manager.add_anchor(&anchor).unwrap();

        let raw = std::fs::read_to_string(manager.anchors_dir().join("t1.toml")).unwrap();
        assert!(raw.contains("日常") && raw.contains("ASMR"), "tags 应写入主播 toml: {}", raw);

        let loaded = manager.load().unwrap();
        let t1 = loaded.anchors.iter().find(|a| a.id == "t1").unwrap();
        assert_eq!(t1.tags, vec!["日常", "ASMR"]);

        // 修改后覆盖保存：新 tags 生效
        anchor.tags = vec!["杂谈".to_string()];
        manager.add_anchor(&anchor).unwrap();
        let reloaded = manager.load().unwrap();
        let t1b = reloaded.anchors.iter().find(|a| a.id == "t1").unwrap();
        assert_eq!(t1b.tags, vec!["杂谈"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_anchor_toml_without_tags_loads_with_empty() {
        // 旧版主播 toml 无 tags 字段：读兼容，默认空数组
        let dir = unique_dir("legacy-tags");
        let manager = ConfigManager::new(dir.clone());
        // load() 依赖全局配置存在（否则视作首次运行返回空主播列表）
        manager.save_global(&GlobalConfig::default()).unwrap();
        std::fs::create_dir_all(manager.anchors_dir()).unwrap();
        std::fs::write(
            manager.anchors_dir().join("old.toml"),
            "id = \"old\"\nname = \"旧主播\"\nurl = \"https://m.missevan.com/live/7\"\nroom_id = \"7\"\nenable_check = true\n",
        )
        .unwrap();
        let loaded = manager.load().unwrap();
        let old = loaded.anchors.iter().find(|a| a.id == "old").unwrap();
        assert!(old.tags.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_first_run_detection() {
        let dir = std::env::temp_dir().join("missevan-test-config");
        let _ = std::fs::remove_dir_all(&dir);
        let manager = ConfigManager::new(dir.clone());
        assert!(manager.is_first_run());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn first_run_depends_on_wizard_completed_not_config_existence() {
        // 首次引导：第 3 步写盘后（wizard_completed=false）退出 → 再次启动仍引导
        let dir = std::env::temp_dir().join(format!(
            "missevan-wizard-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let manager = ConfigManager::new(dir.clone());
        let mut cfg = crate::domain::config::model::Config::default();
        cfg.global.wizard_completed = false;
        manager.save_global(&cfg.global).unwrap();
        assert!(manager.is_first_run(), "写盘但未完成引导 → 应视为首次运行");
        // 第 4 步 finish_wizard 置 true → 不再引导
        cfg.global.wizard_completed = true;
        manager.save_global(&cfg.global).unwrap();
        assert!(!manager.is_first_run(), "引导完成 → 不应再进引导");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn old_config_without_wizard_field_is_not_first_run() {
        // 老用户配置无 wizard_completed 字段 → serde default true → 已完成（无回归）
        let dir = std::env::temp_dir().join("missevan-test-config-wizard-old");
        let _ = std::fs::remove_dir_all(&dir);
        let manager = ConfigManager::new(dir.clone());
        std::fs::create_dir_all(manager.global_config_path().parent().unwrap()).unwrap();
        std::fs::write(manager.global_config_path(), "output_dir = \"./recordings\"\n").unwrap();
        assert!(!manager.is_first_run(), "老配置无字段应视为已完成");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn test_save_and_load_global() {
        let dir = std::env::temp_dir().join("missevan-test-config-2");
        let _ = std::fs::remove_dir_all(&dir);
        let manager = ConfigManager::new(dir.clone());

        let config = GlobalConfig {
            output_dir: "/tmp/recordings".to_string(),
            ..Default::default()
        };

        manager.save_global(&config).unwrap();
        let loaded = manager.load().unwrap();
        assert_eq!(loaded.global.output_dir, "/tmp/recordings");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 构造一个含全局+2 主播的临时配置环境，返回 (manager, 目录)
    fn setup_config() -> (ConfigManager, std::path::PathBuf) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "missevan-test-import-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let manager = ConfigManager::new(dir.clone());
        let mut global = GlobalConfig::default();
        global.output_dir = "D:/recordings".to_string();
        global.proxy_password = "secret".to_string();
        manager.save_global(&global).unwrap();
        manager
            .add_anchor(&AnchorConfig {
                id: "a1".to_string(),
                name: "主播A".to_string(),
                url: "https://m.missevan.com/live/1".to_string(),
                room_id: "1".to_string(),
                proxy: None,
                cookie: Some("ck-a".to_string()),
                enable_check: true,
                avatar_url: None,
                tags: vec!["音乐".to_string()],
            })
            .unwrap();
        manager
            .add_anchor(&AnchorConfig {
                id: "a2".to_string(),
                name: "主播B".to_string(),
                url: "https://m.missevan.com/live/2".to_string(),
                room_id: "2".to_string(),
                proxy: None,
                cookie: None,
                enable_check: true,
                avatar_url: None,
                tags: Vec::new(),
            })
            .unwrap();
        (manager, dir)
    }

    #[test]
    fn export_json_blanks_sensitive_fields() {
        let (manager, dir) = setup_config();
        let json = manager.export_json().unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        // 结构：version + global + anchors
        assert_eq!(v["version"], 1);
        assert_eq!(v["global"]["output_dir"], "D:/recordings");
        assert_eq!(v["global"]["proxy_password"], ""); // 敏感字段置空
        let anchors = v["anchors"].as_array().unwrap();
        assert_eq!(anchors.len(), 2);
        assert!(anchors[0]["cookie"].is_null()); // cookie 置空
        assert_eq!(anchors[1]["cookie"].as_str(), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_then_import_replace_roundtrip() {
        let (manager, dir) = setup_config();
        let json = manager.export_json().unwrap();
        // 先清掉一个主播，验证 replace 全替换会把它恢复
        manager.remove_anchor("a2").unwrap();
        let summary = manager.import_json(&json, "replace").unwrap();
        assert_eq!(summary.mode, "replace");
        assert!(summary.global_replaced);
        assert_eq!(summary.anchors_added, 2);
        assert_eq!(summary.anchors_removed, 0);
        let loaded = manager.load().unwrap();
        assert_eq!(loaded.anchors.len(), 2);
        assert_eq!(loaded.global.output_dir, "D:/recordings");
        // cookie 置空后导入：cookie 恢复为 None（敏感字段不回流）
        assert!(loaded.anchors.iter().all(|a| a.cookie.is_none()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_replace_with_anchors_replaces_local_list() {
        let (manager, dir) = setup_config();
        // 文件只含 1 个新主播（a3）：replace 后本地 a1/a2 被移除
        let json = r#"{
            "version": 1,
            "global": {"output_dir": "E:/new"},
            "anchors": [
                {"id": "a3", "name": "主播C", "url": "https://m.missevan.com/live/3", "room_id": "3", "enable_check": true}
            ]
        }"#;
        let summary = manager.import_json(json, "replace").unwrap();
        assert_eq!(summary.anchors_added, 1);
        assert_eq!(summary.anchors_removed, 2);
        let loaded = manager.load().unwrap();
        assert_eq!(loaded.anchors.len(), 1);
        assert_eq!(loaded.anchors[0].id, "a3");
        assert_eq!(loaded.global.output_dir, "E:/new");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_merge_adds_new_anchors_and_skips_duplicates() {
        let (manager, dir) = setup_config();
        let json = r#"{
            "version": 1,
            "global": {"output_dir": "F:/merged", "notifications_enabled": false},
            "anchors": [
                {"id": "a1", "name": "主播A-改", "url": "https://m.missevan.com/live/1", "room_id": "1", "enable_check": true},
                {"id": "a3", "name": "主播C", "url": "https://m.missevan.com/live/3", "room_id": "3", "enable_check": true}
            ]
        }"#;
        let summary = manager.import_json(json, "merge").unwrap();
        // 重复 id a1 跳过（保留本地）；a3 新增
        assert_eq!(summary.anchors_added, 1);
        assert_eq!(summary.anchors_skipped, 1);
        let loaded = manager.load().unwrap();
        assert_eq!(loaded.anchors.len(), 3);
        let a1 = loaded.anchors.iter().find(|a| a.id == "a1").unwrap();
        assert_eq!(a1.name, "主播A"); // 本地保留，不被文件覆盖
        // global 字段级合并：output_dir 与 notifications_enabled 来自文件，其余保留本地
        assert_eq!(loaded.global.output_dir, "F:/merged");
        assert!(!loaded.global.notifications_enabled);
        assert_eq!(loaded.global.check_interval_secs, 120); // 本地默认保留
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── H2：主播 id 白名单校验 ──

    #[test]
    fn validate_anchor_id_accepts_uuid_and_simple_ids() {
        // UUID（前端正常形态）
        assert!(validate_anchor_id("a1b2c3d4-1234-5678-9abc-def012345678").is_ok());
        // 现有泛化 id
        assert!(validate_anchor_id("a1").is_ok());
        assert!(validate_anchor_id("2").is_ok());
        assert!(validate_anchor_id("ABC_123-xY").is_ok());
        // 边界长度 64
        assert!(validate_anchor_id(&"x".repeat(64)).is_ok());
    }

    #[test]
    fn validate_anchor_id_rejects_paths_and_invalid_chars() {
        assert!(validate_anchor_id("").is_err());
        assert!(validate_anchor_id("..").is_err());
        assert!(validate_anchor_id("../../evil").is_err());
        assert!(validate_anchor_id(r"..\..\evil").is_err());
        assert!(validate_anchor_id("a/b").is_err());
        assert!(validate_anchor_id("a\\b").is_err());
        assert!(validate_anchor_id("a b").is_err());
        assert!(validate_anchor_id("主播A").is_err()); // 非 ASCII 拒绝
        assert!(validate_anchor_id("a.toml").is_err());
        assert!(validate_anchor_id(&"x".repeat(65)).is_err());
    }

    #[test]
    fn add_anchor_rejects_path_traversal_id() {
        let (manager, dir) = setup_config();
        let anchor = AnchorConfig {
            id: "../../../../Users/admin/Desktop/pwn".into(),
            name: "x".into(),
            url: "https://m.missevan.com/live/1".into(),
            room_id: "1".into(),
            proxy: None,
            cookie: None,
            enable_check: true,
            avatar_url: None,
            tags: Vec::new(),
        };
        let err = manager.add_anchor(&anchor).unwrap_err();
        assert!(err.message.contains("非法"), "错误信息: {}", err.message);
        // 磁盘未产生任何越界文件（anchors 目录内也不得出现穿越名的文件）
        let anchors_dir = manager.anchors_dir();
        let written: Vec<_> = std::fs::read_dir(&anchors_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(written.len(), 2, "不应写入新文件: {:?}", written);
        // 空 id 同样拒绝
        let empty = AnchorConfig {
            id: "".into(),
            ..anchor.clone()
        };
        assert!(manager.add_anchor(&empty).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_rejects_path_traversal_anchor_id() {
        let (manager, dir) = setup_config();
        let bad = r#"{"global": {}, "anchors": [{"id": "..\\..\\evil", "name": "x", "url": "u", "room_id": "1", "enable_check": true}]}"#;
        assert!(manager.import_json(bad, "merge").is_err());
        let loaded = manager.load().unwrap();
        assert_eq!(loaded.anchors.len(), 2); // 无新增
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_merge_failure_leaves_global_unchanged() {
        let (manager, dir) = setup_config();
        // 导入文件含空 id 主播：校验失败中止，global 不得落盘（回归：曾先写后验）
        let bad = r#"{
            "version": 1,
            "global": {"output_dir": "H:/should-not-write"},
            "anchors": [
                {"id": "", "name": "x", "url": "u", "room_id": "1", "enable_check": true}
            ]
        }"#;
        assert!(manager.import_json(bad, "merge").is_err());
        let loaded = manager.load().unwrap();
        // 关键字段与导入前一致（setup_config 写入的 output_dir）
        assert_eq!(loaded.global.output_dir, "D:/recordings");
        assert_eq!(loaded.global.proxy_password, "secret");
        // 主播列表也未受影响
        assert_eq!(loaded.anchors.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_flat_format_keeps_anchors_untouched() {
        let (manager, dir) = setup_config();
        // 扁平式：仅 global（前端设置页当前导出形态），不含 anchors
        let json = r#"{"output_dir": "G:/flat", "proxy_password": "", "record_format": "mp3"}"#;
        let summary = manager.import_json(json, "replace").unwrap();
        assert_eq!(summary.anchors_added, 0);
        let loaded = manager.load().unwrap();
        assert_eq!(loaded.anchors.len(), 2); // 主播不动
        assert_eq!(loaded.global.output_dir, "G:/flat");
        assert_eq!(loaded.global.record_format, "mp3");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_rejects_invalid_json_and_bad_mode() {
        let (manager, dir) = setup_config();
        assert!(manager.import_json("not json", "replace").is_err());
        assert!(manager.import_json(r#"{"output_dir": 123}"#, "replace").is_err());
        assert!(manager.import_json(r#"{"output_dir": "x"}"#, "upsert").is_err());
        // 主播列表含空 id：写入前中止
        let bad = r#"{"global": {}, "anchors": [{"id": "", "name": "x", "url": "u", "room_id": "1", "enable_check": true}]}"#;
        assert!(manager.import_json(bad, "merge").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_replace_with_duplicate_anchor_ids_keeps_first() {
        let (manager, dir) = setup_config();
        let json = r#"{
            "global": {},
            "anchors": [
                {"id": "dup", "name": "first", "url": "u1", "room_id": "1", "enable_check": true},
                {"id": "dup", "name": "second", "url": "u2", "room_id": "2", "enable_check": true}
            ]
        }"#;
        let summary = manager.import_json(json, "replace").unwrap();
        assert_eq!(summary.anchors_added, 1);
        assert_eq!(summary.anchors_skipped, 1);
        let loaded = manager.load().unwrap();
        assert_eq!(loaded.anchors.len(), 1);
        assert_eq!(loaded.anchors[0].name, "first");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_merge_with_duplicate_anchor_ids_keeps_first() {
        let (manager, dir) = setup_config();
        let json = r#"{
            "global": {},
            "anchors": [
                {"id": "dup", "name": "first", "url": "u1", "room_id": "1", "enable_check": true},
                {"id": "dup", "name": "second", "url": "u2", "room_id": "2", "enable_check": true}
            ]
        }"#;
        let summary = manager.import_json(json, "merge").unwrap();
        // 文件内重复 id：只取首个 → 新增 1 条，重复 1 条计 skipped
        assert_eq!(summary.anchors_added, 1);
        assert_eq!(summary.anchors_skipped, 1);
        let loaded = manager.load().unwrap();
        // 本地 a1/a2 + 文件首个 dup：共 3 条
        assert_eq!(loaded.anchors.len(), 3);
        let dup = loaded.anchors.iter().find(|a| a.id == "dup").unwrap();
        assert_eq!(dup.name, "first"); // 磁盘保留首个，不被 second 覆盖
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── M1 审查跟进：record_format 白名单 + 并发原子写 ──

    #[test]
    fn save_global_rejects_invalid_record_format() {
        let dir = unique_dir("fmt-whitelist");
        let manager = ConfigManager::new(dir.clone());
        // 路径穿越形态（M1 审查注入向量）与非法值一律拒绝，且不落盘
        for bad in [
            "../../evil",
            "..\\..\\evil",
            "m4a/../pwn",
            "flac",
            "aac",
            "M4A",
            "",
        ] {
            let mut cfg = GlobalConfig::default();
            cfg.record_format = bad.to_string();
            let err = manager.save_global(&cfg).unwrap_err();
            assert!(err.message.contains("录制格式"), "错误信息: {}", err.message);
        }
        assert!(!manager.global_config_path().exists(), "非法格式不得落盘");
        // 白名单内值正常落盘，加载一致
        for good in ["m4a", "mp3"] {
            let mut cfg = GlobalConfig::default();
            cfg.record_format = good.to_string();
            manager.save_global(&cfg).unwrap();
            assert_eq!(manager.load().unwrap().global.record_format, good);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_rejects_invalid_record_format_without_writing() {
        let (manager, dir) = setup_config();
        let bad = r#"{"global": {"record_format": "../../pwn"}, "anchors": []}"#;
        assert!(
            manager.import_json(bad, "replace").is_err(),
            "replace 模式应拒绝非法 record_format"
        );
        assert!(
            manager.import_json(bad, "merge").is_err(),
            "merge 模式应拒绝非法 record_format"
        );
        // 未落盘：原配置保持不变（输出目录、格式均未被污染）
        let loaded = manager.load().unwrap();
        assert_eq!(loaded.global.record_format, "m4a");
        assert_eq!(loaded.global.output_dir, "D:/recordings");
        assert_eq!(loaded.anchors.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn concurrent_saves_never_lose_config_file() {
        // 并发写冒烟回归（M1）：竞态本身难以稳定复现，此测试验证锁 + 唯一 tmp +
        // rename 直替后的不变量——并发 save_global 结束后 config.toml 必须存在
        // 且内容为某个写者完整写入的值（曾可被 remove/rename 交错删成缺失）。
        let dir = unique_dir("concurrent");
        let manager = ConfigManager::new(dir.clone());
        manager.save_global(&GlobalConfig::default()).unwrap(); // 预写初始配置

        std::thread::scope(|s| {
            let manager_ref = &manager; // 引用是 Copy，各闭包各持一份
            for t in 0..8u32 {
                s.spawn(move || {
                    for i in 0..25u32 {
                        let mut cfg = GlobalConfig::default();
                        cfg.output_dir = format!("C:/rec-{}-{}", t, i);
                        manager_ref.save_global(&cfg).unwrap();
                    }
                });
            }
        });

        let raw = std::fs::read_to_string(manager.global_config_path())
            .unwrap_or_else(|e| panic!("config.toml 不得缺失（并发写竞态）: {}", e));
        let parsed: GlobalConfig =
            toml::from_str(&raw).unwrap_or_else(|e| panic!("config.toml 必须是完整可解析内容: {}", e));
        assert!(
            parsed.output_dir.starts_with("C:/rec-"),
            "应为某个写者最后写入的完整值: {}",
            parsed.output_dir
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_all_removes_config_dir_and_resets_first_run() {
        let (manager, dir) = setup_config();
        assert!(!manager.is_first_run());
        manager.delete_all().unwrap();
        assert!(manager.is_first_run());
        assert!(!dir.exists());
        // 幂等：再次删除成功
        manager.delete_all().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
