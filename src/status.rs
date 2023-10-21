use veilid_core::{SharedSecret, VeilidAPI, Crypto, CRYPTO_KIND_VLD0, CryptoSystem, Nonce, CryptoSystemVersion};

use crate::other_err;
use crate::proto::{decode_node_status_message, encode_node_status_message, NodeStatus};
use crate::error::Result;

pub struct StatusManager {
    crypto: Crypto,
}

impl StatusManager {
    pub fn encode(
        &self,
        node_status: &NodeStatus,
        secret: Option<SharedSecret>,
    ) -> Result<Vec<u8>> {
        let plaintext = encode_node_status_message(node_status)?;
        let result = match secret {
            Some(ref s) => {
                let c = self.crypto_system()?;
                let nonce = c.random_nonce();
                let ciphertext = c.encrypt_aead(plaintext.as_slice(), &nonce, s, None)?;
                let mut result = nonce.to_vec();
                result.extend(ciphertext);
                result
            }
            None => plaintext,
        };
        Ok(result)
    }

    pub fn decode(&self, msg_bytes: &[u8], secret: Option<SharedSecret>) -> Result<NodeStatus> {
        let plaintext = match secret {
            Some(ref s) => {
                let c = self.crypto_system()?;
                let nonce = Nonce::new(msg_bytes[0..24].try_into().map_err(other_err)?);
                let ciphertext = &msg_bytes[24..];
                c.decrypt_aead(ciphertext, &nonce, s, None)?
            }
            None => msg_bytes.to_vec(),
        };
        Ok(decode_node_status_message(plaintext.as_slice())?)
    }

    fn crypto_system(&self) -> Result<CryptoSystemVersion> {
        Ok(self.crypto.get(CRYPTO_KIND_VLD0).ok_or(other_err("missing VLD0"))?)
    }
}
