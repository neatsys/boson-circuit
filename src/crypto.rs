use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

// Hashed based digest deriving solution
// There's no well known solution for deriving digest methods to general
// structural data i.e. structs and enums (as far as I know), which means to
// compute digest for a structural data e.g. message type, one has to do either:
//   specify the tranversal manually
//   derive `Hash` and make use of it
//   derive `Serialize` and make use of it
//   derive `BorshSerialize`, which is similar to `Serialize` but has been
//   claimed to be specially designed for this use case
// currently the second approach is take. the benefit is `Hash` semantic
// guarantees the desired reproducibility, and the main problem is the lack of
// cross-platform compatibility, which is hardly concerned in this codebase
// since it is written for benchmarks performed on unified systems and machines.
// nevertheless, I manually addressed the endianness problem below

pub trait DigestHasher {
    fn write(&mut self, bytes: &[u8]);
}

impl DigestHasher for Sha256 {
    fn write(&mut self, bytes: &[u8]) {
        self.update(bytes)
    }
}

struct ImplHasher<'a, T>(&'a mut T);

impl<T: DigestHasher> Hasher for ImplHasher<'_, T> {
    fn write(&mut self, bytes: &[u8]) {
        self.0.write(bytes)
    }

    fn write_u16(&mut self, i: u16) {
        self.0.write(&i.to_le_bytes())
    }

    fn write_u32(&mut self, i: u32) {
        self.0.write(&i.to_le_bytes())
    }

    fn write_u64(&mut self, i: u64) {
        self.0.write(&i.to_le_bytes())
    }

    fn write_usize(&mut self, i: usize) {
        self.0.write(&i.to_le_bytes())
    }

    fn write_i16(&mut self, i: i16) {
        self.0.write(&i.to_le_bytes())
    }

    fn write_i32(&mut self, i: i32) {
        self.0.write(&i.to_le_bytes())
    }

    fn write_i64(&mut self, i: i64) {
        self.0.write(&i.to_le_bytes())
    }

    fn write_isize(&mut self, i: isize) {
        self.0.write(&i.to_le_bytes())
    }

    fn finish(&self) -> u64 {
        unimplemented!()
    }
}

pub trait DigestHash: Hash {
    fn hash(&self, state: &mut impl DigestHasher) {
        Hash::hash(self, &mut ImplHasher(state))
    }

    fn sha256(&self) -> [u8; 32] {
        let mut state = Sha256::new();
        DigestHash::hash(self, &mut state);
        state.finalize().into()
    }
}

impl<T: Hash> DigestHash for T {}

#[derive(Debug, Clone)]
pub struct Crypto<I> {
    secret_key: secp256k1::SecretKey,
    public_keys: HashMap<I, secp256k1::PublicKey>,
    secp: secp256k1::Secp256k1<secp256k1::All>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signature(secp256k1::ecdsa::Signature);

#[derive(Debug, Clone, Serialize, Deserialize, derive_more::Deref)]
pub struct Signed<M> {
    #[deref]
    inner: M,
    signature: Signature,
}

impl<M> Signed<M> {
    pub fn into_inner(self) -> M {
        self.inner
    }
}

impl<I> Crypto<I> {
    pub fn new(
        secret_key: secp256k1::SecretKey,
        public_keys: HashMap<I, secp256k1::PublicKey>,
    ) -> Self {
        Self {
            secret_key,
            public_keys,
            secp: secp256k1::Secp256k1::new(),
        }
    }

    pub fn sign<M: DigestHash>(&self, message: M) -> Signed<M> {
        let digest = secp256k1::Message::from_digest(message.sha256());
        Signed {
            inner: message,
            signature: Signature(self.secp.sign_ecdsa(&digest, &self.secret_key)),
        }
    }

    pub fn verify<M: DigestHash>(&self, index: &I, signed: &Signed<M>) -> anyhow::Result<()>
    where
        I: Eq + Hash,
    {
        let Some(public_key) = self.public_keys.get(index) else {
            anyhow::bail!("no identifier for index")
        };
        let digest = secp256k1::Message::from_digest(signed.inner.sha256());
        self.secp
            .verify_ecdsa(&digest, &signed.signature.0, public_key)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn struct_digest() {
        #[derive(Hash)]
        struct Foo {
            a: u32,
            bs: Vec<u8>,
        }
        let foo = Foo {
            a: 42,
            bs: b"hello".to_vec(),
        };
        assert_ne!(foo.sha256(), <[_; 32]>::default());
    }
}
