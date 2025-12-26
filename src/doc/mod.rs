pub mod author;
pub mod authors;
pub mod document;
pub mod documents;
pub mod service;

pub mod identity {
    use std::fmt;
    use std::hash::Hash;

    #[derive(Clone, Debug)]
    pub struct PrivateKey(pub(crate) p2panda_core::PrivateKey);

    impl Default for PrivateKey {
        fn default() -> Self {
            Self::new()
        }
    }

    impl PrivateKey {
        pub fn new() -> PrivateKey {
            PrivateKey(p2panda_core::PrivateKey::new())
        }

        pub fn public_key(&self) -> PublicKey {
            PublicKey(self.0.public_key())
        }

        pub fn as_bytes(&self) -> &[u8] {
            self.0.as_bytes().as_slice()
        }
    }

    impl TryFrom<&[u8]> for PrivateKey {
        type Error = p2panda_core::IdentityError;

        fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
            Ok(PrivateKey(p2panda_core::PrivateKey::try_from(value)?))
        }
    }

    impl<'a> From<&'a PrivateKey> for &'a [u8] {
        fn from(value: &PrivateKey) -> &[u8] {
            value.0.as_bytes().as_slice()
        }
    }

    impl fmt::Display for PrivateKey {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            fmt::Display::fmt(&self.0, f)
        }
    }

    #[derive(Clone, Debug, PartialEq, Hash, Eq)]
    pub struct PublicKey(pub(crate) p2panda_core::PublicKey);

    impl fmt::Display for PublicKey {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            fmt::Display::fmt(&self.0, f)
        }
    }

    impl<'a> From<&'a PublicKey> for &'a [u8] {
        fn from(value: &PublicKey) -> &[u8] {
            value.0.as_bytes().as_slice()
        }
    }

    impl PublicKey {
        pub fn as_bytes(&self) -> &[u8] {
            self.0.as_bytes().as_slice()
        }
    }
}
