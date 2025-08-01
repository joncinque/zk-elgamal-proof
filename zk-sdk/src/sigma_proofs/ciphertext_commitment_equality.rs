//! The ciphertext-commitment equality sigma proof system.
//!
//! A ciphertext-commitment equality proof is defined with respect to a twisted ElGamal ciphertext
//! and a Pedersen commitment. The proof certifies that a given ciphertext and a commitment pair
//! encrypts/encodes the same message. To generate the proof, a prover must provide the decryption
//! key for the first ciphertext and the Pedersen opening for the commitment.
//!
//! The protocol guarantees computational soundness (by the hardness of discrete log) and perfect
//! zero-knowledge in the random oracle model.

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;
#[cfg(not(target_os = "solana"))]
use {
    crate::{
        encryption::{
            elgamal::{ElGamalCiphertext, ElGamalKeypair, ElGamalPubkey},
            pedersen::{PedersenCommitment, PedersenOpening, G, H},
        },
        sigma_proofs::{canonical_scalar_from_optional_slice, ristretto_point_from_optional_slice},
        UNIT_LEN,
    },
    curve25519_dalek::traits::MultiscalarMul,
    rand::rngs::OsRng,
    zeroize::Zeroize,
};
use {
    crate::{
        sigma_proofs::errors::{EqualityProofVerificationError, SigmaProofVerificationError},
        transcript::TranscriptProtocol,
    },
    curve25519_dalek::{
        ristretto::{CompressedRistretto, RistrettoPoint},
        scalar::Scalar,
        traits::{IsIdentity, VartimeMultiscalarMul},
    },
    merlin::Transcript,
};

/// Byte length of a ciphertext-commitment equality proof.
const CIPHERTEXT_COMMITMENT_EQUALITY_PROOF_LEN: usize = UNIT_LEN * 6;

/// Equality proof.
///
/// Contains all the elliptic curve and scalar components that make up the sigma protocol.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
#[allow(non_snake_case)]
#[derive(Clone)]
pub struct CiphertextCommitmentEqualityProof {
    Y_0: CompressedRistretto,
    Y_1: CompressedRistretto,
    Y_2: CompressedRistretto,
    z_s: Scalar,
    z_x: Scalar,
    z_r: Scalar,
}

#[allow(non_snake_case)]
#[cfg(not(target_os = "solana"))]
impl CiphertextCommitmentEqualityProof {
    /// Creates a ciphertext-commitment equality proof.
    ///
    /// The function does *not* hash the public key, ciphertext, or commitment into the transcript.
    /// For security, the caller (the main protocol) should hash these public components prior to
    /// invoking this constructor.
    ///
    /// This function is randomized. It uses `OsRng` internally to generate random scalars.
    ///
    /// Note that the proof constructor does not take the actual Pedersen commitment as input; it
    /// takes the associated Pedersen opening instead.
    ///
    /// * `keypair` - The ElGamal keypair associated with the first to be proved
    /// * `ciphertext` - The main ElGamal ciphertext to be proved
    /// * `opening` - The opening associated with the main Pedersen commitment to be proved
    /// * `amount` - The message associated with the ElGamal ciphertext and Pedersen commitment
    /// * `transcript` - The transcript that does the bookkeeping for the Fiat-Shamir heuristic
    pub fn new(
        keypair: &ElGamalKeypair,
        ciphertext: &ElGamalCiphertext,
        opening: &PedersenOpening,
        amount: u64,
        transcript: &mut Transcript,
    ) -> Self {
        transcript.ciphertext_commitment_equality_proof_domain_separator();

        // extract the relevant scalar and Ristretto points from the inputs
        let P = keypair.pubkey().get_point();
        let D = ciphertext.handle.get_point();

        let s = keypair.secret().get_scalar();
        let mut x = Scalar::from(amount);
        let r = opening.get_scalar();

        // generate random masking factors that also serves as nonces
        let mut y_s = Scalar::random(&mut OsRng);
        let mut y_x = Scalar::random(&mut OsRng);
        let mut y_r = Scalar::random(&mut OsRng);

        let Y_0 = (&y_s * P).compress();
        let Y_1 = RistrettoPoint::multiscalar_mul(vec![&y_x, &y_s], vec![&G, D]).compress();
        let Y_2 = RistrettoPoint::multiscalar_mul(vec![&y_x, &y_r], vec![&G, &(*H)]).compress();

        // record masking factors in the transcript
        transcript.append_point(b"Y_0", &Y_0);
        transcript.append_point(b"Y_1", &Y_1);
        transcript.append_point(b"Y_2", &Y_2);

        let c = transcript.challenge_scalar(b"c");
        transcript.challenge_scalar(b"w");

        // compute the masked values
        let z_s = &(&c * s) + &y_s;
        let z_x = &(&c * &x) + &y_x;
        let z_r = &(&c * r) + &y_r;

        // zeroize random scalars
        x.zeroize();
        y_s.zeroize();
        y_x.zeroize();
        y_r.zeroize();

        CiphertextCommitmentEqualityProof {
            Y_0,
            Y_1,
            Y_2,
            z_s,
            z_x,
            z_r,
        }
    }

    /// Verifies a ciphertext-commitment equality proof.
    ///
    /// * `pubkey` - The ElGamal pubkey associated with the ciphertext to be proved
    /// * `ciphertext` - The main ElGamal ciphertext to be proved
    /// * `commitment` - The main Pedersen commitment to be proved
    /// * `transcript` - The transcript that does the bookkeeping for the Fiat-Shamir heuristic
    pub fn verify(
        self,
        pubkey: &ElGamalPubkey,
        ciphertext: &ElGamalCiphertext,
        commitment: &PedersenCommitment,
        transcript: &mut Transcript,
    ) -> Result<(), EqualityProofVerificationError> {
        transcript.ciphertext_commitment_equality_proof_domain_separator();

        // extract the relevant scalar and Ristretto points from the inputs
        let P = pubkey.get_point();
        let C_ciphertext = ciphertext.commitment.get_point();
        let D = ciphertext.handle.get_point();
        let C_commitment = commitment.get_point();

        // include Y_0, Y_1, Y_2 to transcript and extract challenges
        transcript.validate_and_append_point(b"Y_0", &self.Y_0)?;
        transcript.validate_and_append_point(b"Y_1", &self.Y_1)?;
        transcript.validate_and_append_point(b"Y_2", &self.Y_2)?;

        let c = transcript.challenge_scalar(b"c");

        transcript.append_scalar(b"z_s", &self.z_s);
        transcript.append_scalar(b"z_x", &self.z_x);
        transcript.append_scalar(b"z_r", &self.z_r);
        let w = transcript.challenge_scalar(b"w"); // w used for batch verification
        let ww = &w * &w;

        let w_negated = -&w;
        let ww_negated = -&ww;

        // check that the required algebraic condition holds
        let Y_0 = self
            .Y_0
            .decompress()
            .ok_or(SigmaProofVerificationError::Deserialization)?;
        let Y_1 = self
            .Y_1
            .decompress()
            .ok_or(SigmaProofVerificationError::Deserialization)?;
        let Y_2 = self
            .Y_2
            .decompress()
            .ok_or(SigmaProofVerificationError::Deserialization)?;

        let check = RistrettoPoint::vartime_multiscalar_mul(
            vec![
                &self.z_s,           // z_s
                &(-&c),              // -c
                &(-&Scalar::ONE),    // -identity
                &(&w * &self.z_x),   // w * z_x
                &(&w * &self.z_s),   // w * z_s
                &(&w_negated * &c),  // -w * c
                &w_negated,          // -w
                &(&ww * &self.z_x),  // ww * z_x
                &(&ww * &self.z_r),  // ww * z_r
                &(&ww_negated * &c), // -ww * c
                &ww_negated,         // -ww
            ],
            vec![
                P,            // P
                &(*H),        // H
                &Y_0,         // Y_0
                &G,           // G
                D,            // D
                C_ciphertext, // C_ciphertext
                &Y_1,         // Y_1
                &G,           // G
                &(*H),        // H
                C_commitment, // C_commitment
                &Y_2,         // Y_2
            ],
        );

        if check.is_identity() {
            Ok(())
        } else {
            Err(SigmaProofVerificationError::AlgebraicRelation.into())
        }
    }

    pub fn to_bytes(&self) -> [u8; CIPHERTEXT_COMMITMENT_EQUALITY_PROOF_LEN] {
        let mut buf = [0_u8; CIPHERTEXT_COMMITMENT_EQUALITY_PROOF_LEN];
        let mut chunks = buf.chunks_mut(UNIT_LEN);
        chunks.next().unwrap().copy_from_slice(self.Y_0.as_bytes());
        chunks.next().unwrap().copy_from_slice(self.Y_1.as_bytes());
        chunks.next().unwrap().copy_from_slice(self.Y_2.as_bytes());
        chunks.next().unwrap().copy_from_slice(self.z_s.as_bytes());
        chunks.next().unwrap().copy_from_slice(self.z_x.as_bytes());
        chunks.next().unwrap().copy_from_slice(self.z_r.as_bytes());
        buf
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, EqualityProofVerificationError> {
        let mut chunks = bytes.chunks(UNIT_LEN);
        let Y_0 = ristretto_point_from_optional_slice(chunks.next())?;
        let Y_1 = ristretto_point_from_optional_slice(chunks.next())?;
        let Y_2 = ristretto_point_from_optional_slice(chunks.next())?;
        let z_s = canonical_scalar_from_optional_slice(chunks.next())?;
        let z_x = canonical_scalar_from_optional_slice(chunks.next())?;
        let z_r = canonical_scalar_from_optional_slice(chunks.next())?;

        Ok(CiphertextCommitmentEqualityProof {
            Y_0,
            Y_1,
            Y_2,
            z_s,
            z_x,
            z_r,
        })
    }
}

#[cfg(test)]
mod test {
    use {
        super::*,
        crate::{
            encryption::{
                elgamal::ElGamalSecretKey,
                pedersen::Pedersen,
                pod::{
                    elgamal::{PodElGamalCiphertext, PodElGamalPubkey},
                    pedersen::PodPedersenCommitment,
                },
            },
            sigma_proofs::pod::PodCiphertextCommitmentEqualityProof,
        },
        std::str::FromStr,
    };

    #[test]
    fn test_ciphertext_commitment_equality_proof_correctness() {
        // success case
        let keypair = ElGamalKeypair::new_rand();
        let message: u64 = 55;

        let ciphertext = keypair.pubkey().encrypt(message);
        let (commitment, opening) = Pedersen::new(message);

        let mut prover_transcript = Transcript::new(b"Test");
        let mut verifier_transcript = Transcript::new(b"Test");

        let proof = CiphertextCommitmentEqualityProof::new(
            &keypair,
            &ciphertext,
            &opening,
            message,
            &mut prover_transcript,
        );

        proof
            .verify(
                keypair.pubkey(),
                &ciphertext,
                &commitment,
                &mut verifier_transcript,
            )
            .unwrap();

        // fail case: encrypted and committed messages are different
        let keypair = ElGamalKeypair::new_rand();
        let encrypted_message: u64 = 55;
        let committed_message: u64 = 77;

        let ciphertext = keypair.pubkey().encrypt(encrypted_message);
        let (commitment, opening) = Pedersen::new(committed_message);

        let mut prover_transcript = Transcript::new(b"Test");
        let mut verifier_transcript = Transcript::new(b"Test");

        let proof = CiphertextCommitmentEqualityProof::new(
            &keypair,
            &ciphertext,
            &opening,
            encrypted_message,
            &mut prover_transcript,
        );

        assert!(proof
            .verify(
                keypair.pubkey(),
                &ciphertext,
                &commitment,
                &mut verifier_transcript
            )
            .is_err());
    }

    #[test]
    fn test_ciphertext_commitment_equality_proof_edge_cases() {
        // if ElGamal public key zero (public key is invalid), then the proof should always reject
        let public = ElGamalPubkey::try_from([0u8; 32].as_slice()).unwrap();
        let secret = ElGamalSecretKey::new_rand();

        let elgamal_keypair = ElGamalKeypair::new_for_tests(public, secret);

        let message: u64 = 55;
        let ciphertext = elgamal_keypair.pubkey().encrypt(message);
        let (commitment, opening) = Pedersen::new(message);

        let mut prover_transcript = Transcript::new(b"Test");
        let mut verifier_transcript = Transcript::new(b"Test");

        let proof = CiphertextCommitmentEqualityProof::new(
            &elgamal_keypair,
            &ciphertext,
            &opening,
            message,
            &mut prover_transcript,
        );

        assert!(proof
            .verify(
                elgamal_keypair.pubkey(),
                &ciphertext,
                &commitment,
                &mut verifier_transcript
            )
            .is_err());

        // if ciphertext is all-zero (valid commitment of 0) and commitment is also all-zero, then
        // the proof should still accept
        let elgamal_keypair = ElGamalKeypair::new_rand();

        let message: u64 = 0;
        let ciphertext = ElGamalCiphertext::from_bytes(&[0u8; 64]).unwrap();
        let commitment = PedersenCommitment::from_bytes(&[0u8; 32]).unwrap();
        let opening = PedersenOpening::from_bytes(&[0u8; 32]).unwrap();

        let mut prover_transcript = Transcript::new(b"Test");
        let mut verifier_transcript = Transcript::new(b"Test");

        let proof = CiphertextCommitmentEqualityProof::new(
            &elgamal_keypair,
            &ciphertext,
            &opening,
            message,
            &mut prover_transcript,
        );

        proof
            .verify(
                elgamal_keypair.pubkey(),
                &ciphertext,
                &commitment,
                &mut verifier_transcript,
            )
            .unwrap();

        // if commitment is all-zero and the ciphertext is a correct encryption of 0, then the
        // proof should still accept
        let elgamal_keypair = ElGamalKeypair::new_rand();

        let message: u64 = 0;
        let ciphertext = elgamal_keypair.pubkey().encrypt(message);
        let commitment = PedersenCommitment::from_bytes(&[0u8; 32]).unwrap();
        let opening = PedersenOpening::from_bytes(&[0u8; 32]).unwrap();

        let mut prover_transcript = Transcript::new(b"Test");
        let mut verifier_transcript = Transcript::new(b"Test");

        let proof = CiphertextCommitmentEqualityProof::new(
            &elgamal_keypair,
            &ciphertext,
            &opening,
            message,
            &mut prover_transcript,
        );

        proof
            .verify(
                elgamal_keypair.pubkey(),
                &ciphertext,
                &commitment,
                &mut verifier_transcript,
            )
            .unwrap();

        // if ciphertext is all zero and commitment correctly encodes 0, then the proof should
        // still accept
        let elgamal_keypair = ElGamalKeypair::new_rand();

        let message: u64 = 0;
        let ciphertext = ElGamalCiphertext::from_bytes(&[0u8; 64]).unwrap();
        let (commitment, opening) = Pedersen::new(message);

        let mut prover_transcript = Transcript::new(b"Test");
        let mut verifier_transcript = Transcript::new(b"Test");

        let proof = CiphertextCommitmentEqualityProof::new(
            &elgamal_keypair,
            &ciphertext,
            &opening,
            message,
            &mut prover_transcript,
        );

        proof
            .verify(
                elgamal_keypair.pubkey(),
                &ciphertext,
                &commitment,
                &mut verifier_transcript,
            )
            .unwrap();
    }

    #[test]
    fn test_ciphertext_commitment_equality_proof_string() {
        let pubkey_str = "JNa7rRrDm35laU7f8HPds1PmHoZEPSHFK/M+aTtEhAk=";
        let pod_pubkey = PodElGamalPubkey::from_str(pubkey_str).unwrap();
        let pubkey: ElGamalPubkey = pod_pubkey.try_into().unwrap();

        let ciphertext_str = "RAXnbQ/DPRlYAWmD+iHRNqMDv7oQcPgQ7OejRzj4bxVy2qOJNziqqDOC7VP3iTW1+z/jckW4smA3EUF7i/r8Rw==";
        let pod_ciphertext = PodElGamalCiphertext::from_str(ciphertext_str).unwrap();
        let ciphertext: ElGamalCiphertext = pod_ciphertext.try_into().unwrap();

        let commitment_str = "ngPTYvbY9P5l6aOfr7bLQiI+0HZsw8GBgiumdW3tNzw=";
        let pod_commitment = PodPedersenCommitment::from_str(commitment_str).unwrap();
        let commitment: PedersenCommitment = pod_commitment.try_into().unwrap();

        let proof_str = "cCZySLxB2XJdGyDvckVBm2OWiXqf7Jf54IFoDuLJ4G+ySj+lh5DbaDMHDhuozQC9tDWtk2mFITuaXOc5Zw3nZ2oEvVYpqv5hN+k5dx9k8/nZKabUCkZwx310z7x4fE4Np5SY9PYia1hkrq9AWq0b3v97XvW1+XCSSxuflvBk5wsdaQQ+ZgcmPnKWKjHfRwmU2k5iVgYzs2VmvZa5E3OWBoM/M2yFNvukY+FCC2YMnspO0c4lNBr/vDFQuHdW0OgJ";
        let pod_proof = PodCiphertextCommitmentEqualityProof::from_str(proof_str).unwrap();
        let proof: CiphertextCommitmentEqualityProof = pod_proof.try_into().unwrap();

        let mut verifier_transcript = Transcript::new(b"Test");

        proof
            .verify(&pubkey, &ciphertext, &commitment, &mut verifier_transcript)
            .unwrap();
    }
}
