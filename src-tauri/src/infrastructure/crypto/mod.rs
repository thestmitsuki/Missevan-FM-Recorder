//! 敏感字段轻量混淆（设计文档 §11.7 / 规格 5.1）
//!
//! 方案：机器特征密钥 XOR + Base64，密文带 `enc:v1:` 前缀。
//! **声明：非强加密**——目标是避免 Cookie / 代理密码以明文落盘
//! （防静态查看、防误提交），不提供防篡改 / 强保密保证。
//!
//! - 密钥：可执行文件路径 + 主机名 + 固定 salt 的 FNV-1a 哈希（32 字节）。
//!   同一台机器 / 同一安装位置内稳定；换机器或换目录后旧密文无法解开，
//!   由调用方（ConfigManager）回退为明文——旧数据仍可读，下次保存时以新密钥重新混淆。
//! - 兼容性：`deobfuscate` 对无前缀 / 损坏密文返回 `Err`；
//!   `deobfuscate_or_plain` 失败时回退为原样返回，旧版明文配置读取不受影响。

use std::fmt;

use crate::tr;

/// 混淆失败错误（调用方决定是否回退明文）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CryptoError(pub String);

impl fmt::Display for CryptoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", tr!("log.crypto_decrypt_failed", msg = self.0))
    }
}

/// 密文前缀：`enc:v1:` + base64(XOR(bytes))
pub const PREFIX: &str = "enc:v1:";

/// 机器特征密钥（32 字节）：
/// FNV-1a 哈希（exe 路径 | 主机名 | 固定 salt），按 counter 扩展至 32 字节。
/// FNV-1a 为自实现稳定算法，不依赖 std hash 的内部实现，跨 Rust 版本不变。
pub fn machine_key() -> [u8; 32] {
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let host = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_default();
    let salt = "missevan-recorder::obfuscate::v1";
    let mut out = [0u8; 32];
    for chunk in 0..4u32 {
        let material = format!("{}|{}|{}|{}", exe, host, salt, chunk);
        let h = fnv1a64(material.as_bytes());
        let start = (chunk as usize) * 8;
        out[start..start + 8].copy_from_slice(&h.to_le_bytes());
    }
    out
}

fn fnv1a64(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in data {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// 混淆：XOR（密钥循环）+ Base64，带 `enc:v1:` 前缀；空串原样返回（不混淆空值）
pub fn obfuscate(s: &str, key: &[u8]) -> String {
    if s.is_empty() {
        return String::new();
    }
    let key = if key.is_empty() { &[0u8] } else { key };
    let bytes = s.as_bytes();
    let mut xored = Vec::with_capacity(bytes.len());
    for (i, b) in bytes.iter().enumerate() {
        xored.push(b ^ key[i % key.len()]);
    }
    use base64::Engine;
    format!(
        "{}{}",
        PREFIX,
        base64::engine::general_purpose::STANDARD.encode(xored)
    )
}

/// 解密：要求 `enc:v1:` 前缀；非法 Base64 / 非 UTF-8 返回 `Err`
pub fn deobfuscate(s: &str, key: &[u8]) -> Result<String, CryptoError> {
    if s.is_empty() {
        return Ok(String::new()); // 空串 ↔ 空串（obfuscate 空串原样返回）
    }
    let payload = s
        .strip_prefix(PREFIX)
        .ok_or_else(|| CryptoError(tr!("log.crypto_missing_prefix").into()))?;
    if payload.is_empty() {
        return Ok(String::new());
    }
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload)
        .map_err(|e| CryptoError(tr!("log.crypto_base64_failed", err = e)))?;
    let key = if key.is_empty() { &[0u8] } else { key };
    let plain: Vec<u8> = bytes
        .iter()
        .enumerate()
        .map(|(i, b)| b ^ key[i % key.len()])
        .collect();
    String::from_utf8(plain).map_err(|_| CryptoError(tr!("log.crypto_utf8_failed").into()))
}

/// 解密或回退原样：`deobfuscate` 失败时返回原文。
/// 兼容旧版明文配置：损坏 / 无前缀的内容按明文处理，不阻断读取
/// （下次保存时以当前密钥重新混淆）。
pub fn deobfuscate_or_plain(s: &str, key: &[u8]) -> String {
    match deobfuscate(s, key) {
        Ok(v) => v,
        Err(_) => {
            // 带 enc:v1: 前缀却解密失败 = 跨机器/跨安装目录的密钥不匹配（或密文损坏）。
            // 回退明文继续读取（读兼容不破坏），但必须告警——否则用户无感知，
            // 下次保存还会对密文整体再次混淆，凭据将永久损坏无法恢复。
            // 无前缀内容 = 旧版明文配置（正常读兼容路径），不刷日志。
            if s.starts_with(PREFIX) {
                tracing::warn!(
                    "config value obfuscated with a different machine key, falling back to plain text"
                );
            }
            s.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &[u8] = b"test-key-1234";

    #[test]
    fn roundtrip_ascii() {
        let s = "cookie123";
        assert_eq!(deobfuscate(&obfuscate(s, KEY), KEY).unwrap(), s);
    }

    #[test]
    fn roundtrip_unicode() {
        let s = "中文 Cookie: 密码·漢字 🎙️ emoji";
        assert_eq!(deobfuscate(&obfuscate(s, KEY), KEY).unwrap(), s);
    }

    #[test]
    fn roundtrip_empty_string() {
        // 空串不混淆：明文空串 ↔ 密文空串
        assert_eq!(obfuscate("", KEY), "");
        assert_eq!(deobfuscate("", KEY).unwrap(), "");
    }

    #[test]
    fn roundtrip_long_string() {
        let s = "x".repeat(10_000) + "尾部";
        let enc = obfuscate(&s, KEY);
        assert_eq!(deobfuscate(&enc, KEY).unwrap(), s);
    }

    #[test]
    fn roundtrip_with_empty_key_is_deterministic() {
        // 空密钥按 [0] 处理：不 panic，可逆
        let s = "secret";
        assert_eq!(deobfuscate(&obfuscate(s, b""), b"").unwrap(), s);
    }

    #[test]
    fn deobfuscate_rejects_plaintext_without_prefix() {
        // 旧版明文配置：无前缀 → Err（由 deobfuscate_or_plain 回退）
        assert!(deobfuscate("mysecret", KEY).is_err());
        assert!(deobfuscate("任意明文内容", KEY).is_err());
    }

    #[test]
    fn deobfuscate_rejects_corrupted_ciphertext() {
        // 前缀存在但内容损坏
        assert!(deobfuscate("enc:v1:not-valid-base64!!!", KEY).is_err());
        // Base64 合法但 XOR 结果非 UTF-8（0xFF 与密钥异或后为非法序列）
        assert!(deobfuscate("enc:v1:////", KEY).is_err());
        // 仅前缀（无内容）→ 空串
        assert_eq!(deobfuscate("enc:v1:", KEY).unwrap(), "");
    }

    #[test]
    fn deobfuscate_or_plain_falls_back_on_failure() {
        assert_eq!(deobfuscate_or_plain("legacy-plaintext", KEY), "legacy-plaintext");
        assert_eq!(deobfuscate_or_plain("enc:v1:broken!!", KEY), "enc:v1:broken!!");
        assert_eq!(deobfuscate_or_plain("", KEY), "");
        // 正常密文走解密
        let enc = obfuscate("real-secret", KEY);
        assert_eq!(deobfuscate_or_plain(&enc, KEY), "real-secret");
    }

    #[test]
    fn ciphertext_hides_plaintext() {
        let s = "super-secret-password";
        let enc = obfuscate(s, KEY);
        assert!(!enc.contains(s), "密文不得包含明文: {}", enc);
        assert!(enc.starts_with(PREFIX));
    }

    #[test]
    fn obfuscate_is_deterministic() {
        let a = obfuscate("same", KEY);
        let b = obfuscate("same", KEY);
        assert_eq!(a, b);
    }

    #[test]
    fn machine_key_is_stable_32_bytes() {
        let k1 = machine_key();
        let k2 = machine_key();
        assert_eq!(k1, k2);
        assert_eq!(k1.len(), 32);
        assert_ne!(k1, [0u8; 32], "密钥不应为全零");
    }

    #[test]
    fn different_keys_produce_different_ciphertexts() {
        let s = "password-12345-abcdefghijklmnop";
        let enc1 = obfuscate(s, b"key-a");
        let enc2 = obfuscate(s, b"key-b");
        assert_ne!(enc1, enc2);
        // 用错密钥无法还原原文
        assert_ne!(deobfuscate_or_plain(&enc1, b"key-b"), s);
    }
}
