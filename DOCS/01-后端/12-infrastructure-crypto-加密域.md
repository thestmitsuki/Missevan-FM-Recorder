# 12 · infrastructure/crypto —— 敏感字段混淆域

> 文件：`src-tauri/src/infrastructure/crypto/mod.rs`

## 1. 职责

敏感字段（Cookie / 代理密码）落盘前的**轻量混淆**，避免明文落盘（防静态查看、防误提交配置）。

## 2. 方案

```
机器特征密钥 XOR + Base64，密文前缀 "enc:v1:"
密钥 = FNV-1a 哈希（exe 路径 | 主机名 | 固定 salt）→ 32 字节
```

- **声明：非强加密**——目标是防静态查看/防误提交，不提供防篡改/强保密保证；
- 密钥绑定「可执行文件路径 + 主机名」：同一台机器 / 同一安装位置内稳定；换机器或换目录后旧密文无法解开 → 调用方（ConfigManager）回退明文，旧数据仍可读，下次保存以新密钥重新混淆。

## 3. API

| 函数 | 说明 |
| --- | --- |
| `obfuscate(plain: &str, key: &[u8]) -> String` | 混淆（`enc:v1:` 前缀） |
| `deobfuscate(cipher: &str, key: &[u8]) -> Result<String, CryptoError>` | 解混淆；无前缀/损坏 → Err |
| `deobfuscate_or_plain(cipher, key) -> String` | 失败回退原样返回（旧版明文配置兼容） |
| `machine_key() -> [u8; 32]` | 机器特征密钥（exe 路径 + 主机名 + salt 的 FNV-1a） |

## 4. 跨模块依赖

| 消费方 | 用途 |
| --- | --- |
| `domain/config/manager.rs` | 保存时混淆 cookie / 代理密码；读取时 deobfuscate_or_plain |
| `api/debug_cmds.rs` | 诊断导出（密文不导出明文） |

## 5. 测试

- 混淆/解混淆往返（正确密钥）；
- 不同密钥不可互解；
- 无前缀明文直接返回（兼容旧配置）；
- 前缀存在性校验。

## 6. 已知陷阱

- **不是加密**：别拿它做认证/防篡改；用户协议与 README 免责声明已明确。
- 换机器/换安装目录后密文失效 → 自动回退明文是**特性**（迁移友好），但意味着旧配置里的敏感字段在该机器上会以明文写回——这是设计接受的权衡。
- 新增敏感字段（如代理认证、第三方 token）落盘前必须走 `obfuscate`，并同步前端（`types/config.ts` 对应字段类型与展示脱敏）。
