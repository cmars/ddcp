use std::array::TryFromSliceError;

use veilid_core::{CryptoSystemVersion, Nonce, SharedSecret, VeilidAPIError, NONCE_LENGTH};

use crate::proto;

use proto::{Decodable, Encodable};

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("{0}")]
    Proto(#[from] proto::Error),
    #[error("{0}")]
    VeilidAPI(#[from] VeilidAPIError),
    #[error("{0}")]
    DecodeFailure(#[from] TryFromSliceError),
}
pub type Result<T> = std::result::Result<T, Error>;

pub struct Crypto {
    crypto_system: CryptoSystemVersion,
    secret_key: SharedSecret,
}

impl Crypto {
    pub fn new(crypto_system: CryptoSystemVersion, secret_key: SharedSecret) -> Crypto {
        Crypto {
            crypto_system,
            secret_key,
        }
    }

    pub fn encode<T: Encodable>(&self, item: T) -> Result<Vec<u8>> {
        let mut message = item.encode()?;
        let nonce = self.crypto_system.random_nonce();
        self.crypto_system
            .encrypt_in_place_aead(&mut message, &nonce, &self.secret_key, None)?;
        Ok(vec![nonce.to_vec(), message].concat())
    }

    pub fn decode<T: Decodable>(&self, message: &[u8]) -> Result<T> {
        let nonce = Nonce::new(message[0..NONCE_LENGTH].try_into()?);
        let mut message = message[NONCE_LENGTH..].to_vec();
        self.crypto_system
            .decrypt_in_place_aead(&mut message, &nonce, &self.secret_key, None)?;
        let result = T::decode(message.as_slice())?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use veilid_core::CRYPTO_KIND_VLD0;

    use crate::proto::Request;
    use crate::tests::api::{setup_api, teardown_api};

    use super::*;

    #[tokio::test]
    async fn roundtrip() {
        let api = setup_api().await;
        let crypto_system = api
            .crypto()
            .expect("crypto")
            .get(CRYPTO_KIND_VLD0)
            .expect("vld0");
        let crypto = Crypto::new(crypto_system.clone(), crypto_system.random_shared_secret());
        let msg_bytes = crypto.encode(Request::Status).expect("encode");
        let decoded = crypto.decode::<Request>(&msg_bytes).expect("ok");
        assert_eq!(Request::Status, decoded);
        teardown_api(api);
    }

}
