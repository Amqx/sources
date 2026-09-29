use aidoku::{Result, alloc::String, error};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

pub fn encrypt(data: &[u8], key: &[u8]) -> Result<String> {
    let mut mac = HmacSha256::new_from_slice(key).map_err(|_| error!("Invalid key length"))?;
    mac.update(data);
    let result = mac.finalize();
    Ok(hex::encode(result.into_bytes()))
}
