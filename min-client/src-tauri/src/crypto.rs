use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit},
    Aes256Gcm,
};
use rand_core::OsRng;
use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::{DateTime, Utc};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use snow::{params::NoiseParams, Builder};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Identity {
    pub id: String,
    pub display_name: String,
    pub public_key: String,
    #[serde(skip_serializing)]
    pub secret_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedEnvelope {
    pub id: Uuid,
    pub nonce: String,
    pub ciphertext: String,
    pub sender_public_key: String,
    pub recipient_id: String,
    pub created_at: DateTime<Utc>,
}

pub fn generate_identity(display_name: String) -> Identity {
    let signing_key = SigningKey::generate(&mut rand_core::OsRng);
    let public = signing_key.verifying_key().to_bytes();
    let id = STANDARD.encode(Sha256::digest(public))[0..16].to_string();

    Identity {
        id,
        display_name,
        public_key: STANDARD.encode(public),
        secret_key: STANDARD.encode(signing_key.to_bytes()),
    }
}

pub fn sign(identity: &Identity, payload: &[u8]) -> anyhow::Result<String> {
    let secret = decode_array::<32>(&identity.secret_key)?;
    let signing_key = SigningKey::from_bytes(&secret);
    Ok(STANDARD.encode(signing_key.sign(payload).to_bytes()))
}

pub fn verify(public_key: &str, payload: &[u8], signature: &str) -> anyhow::Result<()> {
    let public = VerifyingKey::from_bytes(&decode_array::<32>(public_key)?)?;
    let signature = Signature::from_bytes(&decode_array::<64>(signature)?);
    public.verify(payload, &signature)?;
    Ok(())
}

pub fn encrypt_message(
    identity: &Identity,
    recipient_id: String,
    conversation_key_b64: &str,
    body: &str,
) -> anyhow::Result<EncryptedEnvelope> {
    let key = decode_array::<32>(conversation_key_b64)?;
    let cipher = Aes256Gcm::new((&key).into());
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher.encrypt(&nonce, body.as_bytes())?;

    Ok(EncryptedEnvelope {
        id: Uuid::new_v4(),
        nonce: STANDARD.encode(nonce),
        ciphertext: STANDARD.encode(ciphertext),
        sender_public_key: identity.public_key.clone(),
        recipient_id,
        created_at: Utc::now(),
    })
}

pub fn decrypt_message(envelope: &EncryptedEnvelope, conversation_key_b64: &str) -> anyhow::Result<String> {
    let key = decode_array::<32>(conversation_key_b64)?;
    let cipher = Aes256Gcm::new((&key).into());
    let nonce = decode_array::<12>(&envelope.nonce)?;
    let ciphertext = STANDARD.decode(&envelope.ciphertext)?;
    let plaintext = cipher.decrypt((&nonce).into(), ciphertext.as_ref())?;
    Ok(String::from_utf8(plaintext)?)
}

pub fn new_conversation_key() -> String {
    let key = Aes256Gcm::generate_key(&mut OsRng);
    STANDARD.encode(key)
}

pub fn build_noise_initiator() -> anyhow::Result<snow::HandshakeState> {
    let params: NoiseParams = "Noise_XX_25519_AESGCM_SHA256".parse()?;
    Ok(Builder::new(params).build_initiator()?)
}

pub fn build_noise_responder() -> anyhow::Result<snow::HandshakeState> {
    let params: NoiseParams = "Noise_XX_25519_AESGCM_SHA256".parse()?;
    Ok(Builder::new(params).build_responder()?)
}

fn decode_array<const N: usize>(value: &str) -> anyhow::Result<[u8; N]> {
    let bytes = STANDARD.decode(value)?;
    Ok(bytes.try_into().map_err(|_| anyhow::anyhow!("invalid key length"))?)
}
