use std::array::TryFromSliceError;

use veilid_core::{
    CryptoSystemVersion, Nonce, TypedKey, TypedKeyPair, VeilidAPIError, NONCE_LENGTH,
};

use crate::proto::codec;

use codec::{Decodable, Encodable};

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("{0}")]
    Proto(#[from] codec::Error),
    #[error("{0}")]
    VeilidAPI(#[from] VeilidAPIError),
    #[error("{0}")]
    DecodeFailure(#[from] TryFromSliceError),
}
pub type Result<T> = std::result::Result<T, Error>;

pub struct Crypto {
    crypto_system: CryptoSystemVersion,
    sender_key_pair: TypedKeyPair,
    recipient_public_key: TypedKey,
}

impl Crypto {
    pub fn new(
        crypto_system: CryptoSystemVersion,
        sender_key_pair: TypedKeyPair,
        recipient_public_key: TypedKey,
    ) -> Crypto {
        Crypto {
            crypto_system,
            sender_key_pair,
            recipient_public_key,
        }
    }

    pub fn encode<T: Encodable>(&self, item: T) -> Result<Vec<u8>> {
        let mut message = item.encode()?;
        let shared_key = self.crypto_system.cached_dh(
            &self.recipient_public_key.value,
            &self.sender_key_pair.value.secret,
        )?;
        let nonce = self.crypto_system.random_nonce();
        self.crypto_system
            .encrypt_in_place_aead(&mut message, &nonce, &shared_key, None)?;
        Ok(vec![nonce.to_vec(), message].concat())
    }

    pub fn decode<T: Decodable>(&self, message: &[u8]) -> Result<T> {
        let nonce = Nonce::new(message[0..NONCE_LENGTH].try_into()?);
        let mut message = message[NONCE_LENGTH..].to_vec();
        let shared_key = self.crypto_system.cached_dh(
            &self.recipient_public_key.value,
            &self.sender_key_pair.value.secret,
        )?;
        self.crypto_system
            .decrypt_in_place_aead(&mut message, &nonce, &shared_key, None)?;
        let result = T::decode(message.as_slice())?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use veilid_core::{CryptoTyped, CRYPTO_KIND_VLD0};

    use crate::proto::codec::Request;
    use crate::tests::api::{setup_api, teardown_api, TEST_API_MUTEX};

    use super::*;

    #[tokio::test]
    async fn roundtrip() {
        let _lock = TEST_API_MUTEX.lock().expect("lock");

        let api = setup_api().await;
        let crypto_system = api
            .crypto()
            .expect("crypto")
            .get(CRYPTO_KIND_VLD0)
            .expect("vld0");

        let alice_keypair = CryptoTyped {
            kind: CRYPTO_KIND_VLD0,
            value: crypto_system.generate_keypair(),
        };
        let bob_keypair = CryptoTyped {
            kind: CRYPTO_KIND_VLD0,
            value: crypto_system.generate_keypair(),
        };

        let alice_crypto = Crypto::new(
            crypto_system.clone(),
            alice_keypair,
            CryptoTyped {
                kind: bob_keypair.kind,
                value: bob_keypair.value.key,
            },
        );
        let bob_crypto = Crypto::new(
            crypto_system.clone(),
            bob_keypair,
            CryptoTyped {
                kind: alice_keypair.kind,
                value: alice_keypair.value.key,
            },
        );

        let msg_bytes = alice_crypto.encode(Request::Status).expect("encode");
        let decoded = bob_crypto.decode::<Request>(&msg_bytes).expect("ok");
        assert_eq!(Request::Status, decoded);
        teardown_api(api).await;
    }
}
