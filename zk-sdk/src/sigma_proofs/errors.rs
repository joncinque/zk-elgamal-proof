//! Errors related to proving and verifying sigma proofs.
use {crate::errors::TranscriptError, thiserror::Error};

#[derive(Error, Clone, Debug, Eq, PartialEq)]
pub enum SigmaProofVerificationError {
    #[error("required algebraic relation does not hold")]
    AlgebraicRelation,
    #[error("malformed proof")]
    Deserialization,
    #[error("multiscalar multiplication failed")]
    MultiscalarMul,
    #[error("transcript failed to produce a challenge")]
    Transcript(#[from] TranscriptError),
    #[error("public key is the identity")]
    PubkeyIsIdentity,
}

macro_rules! impl_from_transcript_error {
    ($sigma_error_type:ty) => {
        impl From<TranscriptError> for $sigma_error_type {
            fn from(err: TranscriptError) -> Self {
                SigmaProofVerificationError::Transcript(err).into()
            }
        }
    };
}

#[derive(Error, Clone, Debug, Eq, PartialEq)]
#[error("equality proof verification failed: {0}")]
pub struct EqualityProofVerificationError(#[from] pub(crate) SigmaProofVerificationError);
impl_from_transcript_error!(EqualityProofVerificationError);

#[derive(Error, Clone, Debug, Eq, PartialEq)]
#[error("validity proof verification failed: {0}")]
pub struct ValidityProofVerificationError(#[from] pub(crate) SigmaProofVerificationError);
impl_from_transcript_error!(ValidityProofVerificationError);

#[derive(Error, Clone, Debug, Eq, PartialEq)]
#[error("zero-ciphertext proof verification failed: {0}")]
pub struct ZeroCiphertextProofVerificationError(#[from] pub(crate) SigmaProofVerificationError);
impl_from_transcript_error!(ZeroCiphertextProofVerificationError);

#[derive(Error, Clone, Debug, Eq, PartialEq)]
#[error("percentage-with-cap proof verification failed: {0}")]
pub struct PercentageWithCapProofVerificationError(#[from] pub(crate) SigmaProofVerificationError);
impl_from_transcript_error!(PercentageWithCapProofVerificationError);

#[derive(Error, Clone, Debug, Eq, PartialEq)]
#[error("public key validity proof verification failed: {0}")]
pub struct PubkeyValidityProofVerificationError(#[from] pub(crate) SigmaProofVerificationError);
impl_from_transcript_error!(PubkeyValidityProofVerificationError);
