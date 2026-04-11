use aes::cipher::{BlockDecrypt, BlockEncrypt, KeyInit};
use aes::Aes128;
use anyhow::{Context, Result};
use base64::Engine;

#[allow(dead_code)]
const BLOCK_SIZE: usize = 16;

/// AES-128-ECB 解密（PKCS7 去填充）
#[allow(dead_code)]
pub fn aes_ecb_decrypt(ciphertext: &[u8], key_b64: &str) -> Result<Vec<u8>> {
    let key_bytes = base64::engine::general_purpose::STANDARD
        .decode(key_b64)
        .context("解码 AES key 失败")?;

    if key_bytes.len() != 16 {
        anyhow::bail!(
            "AES key 长度无效: 期望 16 字节, 实际 {} 字节",
            key_bytes.len()
        );
    }

    let cipher = Aes128::new_from_slice(&key_bytes)
        .map_err(|e| anyhow::anyhow!("创建 AES cipher 失败: {e}"))?;

    let mut plaintext = ciphertext.to_vec();

    // ECB 模式逐块解密
    for chunk in plaintext.chunks_exact_mut(BLOCK_SIZE) {
        let block = aes::Block::from_mut_slice(chunk);
        cipher.decrypt_block(block);
    }

    // PKCS7 去填充
    if let Some(&pad_len) = plaintext.last() {
        let pad_len = pad_len as usize;
        if pad_len > 0 && pad_len <= BLOCK_SIZE && plaintext.len() >= pad_len {
            // 验证填充是否正确
            let start = plaintext.len() - pad_len;
            if plaintext[start..].iter().all(|&b| b == pad_len as u8) {
                plaintext.truncate(start);
            }
        }
    }

    Ok(plaintext)
}

/// AES-128-ECB 加密（PKCS7 填充）
#[allow(dead_code)]
pub fn aes_ecb_encrypt(plaintext: &[u8], key_b64: &str) -> Result<Vec<u8>> {
    let key_bytes = base64::engine::general_purpose::STANDARD
        .decode(key_b64)
        .context("解码 AES key 失败")?;

    if key_bytes.len() != 16 {
        anyhow::bail!(
            "AES key 长度无效: 期望 16 字节, 实际 {} 字节",
            key_bytes.len()
        );
    }

    let cipher = Aes128::new_from_slice(&key_bytes)
        .map_err(|e| anyhow::anyhow!("创建 AES cipher 失败: {e}"))?;

    // PKCS7 填充
    let pad_len = BLOCK_SIZE - (plaintext.len() % BLOCK_SIZE);
    let mut padded = plaintext.to_vec();
    padded.extend(std::iter::repeat_n(pad_len as u8, pad_len));

    // ECB 模式逐块加密
    for chunk in padded.chunks_exact_mut(BLOCK_SIZE) {
        let block = aes::Block::from_mut_slice(chunk);
        cipher.encrypt_block(block);
    }

    Ok(padded)
}

/// 生成随机 AES-128 密钥（返回 base64 编码）
#[allow(dead_code)]
pub fn generate_aes_key() -> String {
    use rand::RngCore;
    let mut key = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut key);
    base64::engine::general_purpose::STANDARD.encode(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let key = generate_aes_key();
        let original = b"Hello, WeChat iLink Bot!";

        let encrypted = aes_ecb_encrypt(original, &key).unwrap();
        let decrypted = aes_ecb_decrypt(&encrypted, &key).unwrap();

        assert_eq!(decrypted, original);
    }

    #[test]
    fn test_encrypt_decrypt_empty() {
        let key = generate_aes_key();
        let original = b"";

        let encrypted = aes_ecb_encrypt(original, &key).unwrap();
        let decrypted = aes_ecb_decrypt(&encrypted, &key).unwrap();

        assert_eq!(decrypted, original);
    }

    #[test]
    fn test_encrypt_decrypt_block_aligned() {
        let key = generate_aes_key();
        let original = b"1234567890123456"; // 正好 16 字节

        let encrypted = aes_ecb_encrypt(original, &key).unwrap();
        let decrypted = aes_ecb_decrypt(&encrypted, &key).unwrap();

        assert_eq!(decrypted, original);
    }
}
