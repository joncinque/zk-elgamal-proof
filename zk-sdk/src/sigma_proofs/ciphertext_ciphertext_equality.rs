//! The ciphertext-ciphertext equality sigma proof system.
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
            pedersen::{PedersenOpening, G, H},
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

/// Byte length of a ciphertext-ciphertext equality proof.
const CIPHERTEXT_CIPHERTEXT_EQUALITY_PROOF_LEN: usize = UNIT_LEN * 7;

/// The ciphertext-ciphertext equality proof.
///
/// Contains all the elliptic curve and scalar components that make up the sigma protocol.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
#[allow(non_snake_case)]
#[derive(Clone)]
pub struct CiphertextCiphertextEqualityProof {
    Y_0: CompressedRistretto,
    Y_1: CompressedRistretto,
    Y_2: CompressedRistretto,
    Y_3: CompressedRistretto,
    z_s: Scalar,
    z_x: Scalar,
    z_r: Scalar,
}

#[allow(non_snake_case)]
#[cfg(not(target_os = "solana"))]
impl CiphertextCiphertextEqualityProof {
    /// Creates a ciphertext-ciphertext equality proof.
    ///
    /// The function does *not* hash the public keys, first ciphertext, or second ciphertext into the transcript.
    /// For security, the caller (the main protocol) should hash these public components prior to
    /// invoking this constructor.
    ///
    /// This function is randomized. It uses `OsRng` internally to generate random scalars.
    ///
    /// * `first_keypair` - The ElGamal keypair associated with the first ciphertext to be proved
    /// * `second_pubkey` - The ElGamal pubkey associated with the second ElGamal ciphertext
    /// * `first_ciphertext` - The first ElGamal ciphertext for which the prover knows a
    ///   decryption key for
    /// * `second_opening` - The opening (randomness) associated with the second ElGamal ciphertext
    /// * `amount` - The message associated with the ElGamal ciphertext and Pedersen commitment
    /// * `transcript` - The transcript that does the bookkeeping for the Fiat-Shamir heuristic
    pub fn new(
        first_keypair: &ElGamalKeypair,
        second_pubkey: &ElGamalPubkey,
        first_ciphertext: &ElGamalCiphertext,
        second_opening: &PedersenOpening,
        amount: u64,
        transcript: &mut Transcript,
    ) -> Self {
        transcript.ciphertext_ciphertext_equality_proof_domain_separator();

        // extract the relevant scalar and Ristretto points from the inputs
        let P_first = first_keypair.pubkey().get_point();
        let D_first = first_ciphertext.handle.get_point();
        let P_second = second_pubkey.get_point();

        let s = first_keypair.secret().get_scalar();
        let mut x = Scalar::from(amount);
        let r = second_opening.get_scalar();

        // generate random masking factors that also serves as nonces
        let mut y_s = Scalar::random(&mut OsRng);
        let mut y_x = Scalar::random(&mut OsRng);
        let mut y_r = Scalar::random(&mut OsRng);

        let Y_0 = (&y_s * P_first).compress();
        let Y_1 = RistrettoPoint::multiscalar_mul(vec![&y_x, &y_s], vec![&G, D_first]).compress();
        let Y_2 = RistrettoPoint::multiscalar_mul(vec![&y_x, &y_r], vec![&G, &(*H)]).compress();
        let Y_3 = (&y_r * P_second).compress();

        // record masking factors in the transcript
        transcript.append_point(b"Y_0", &Y_0);
        transcript.append_point(b"Y_1", &Y_1);
        transcript.append_point(b"Y_2", &Y_2);
        transcript.append_point(b"Y_3", &Y_3);

        let c = transcript.challenge_scalar(b"c");
        transcript.challenge_scalar(b"w");

        // compute the masked values
        let z_s = &(&c * s) + &y_s;
        let z_x = &(&c * &x) + &y_x;
        let z_r = &(&c * r) + &y_r;

        // zeroize all sensitive non-reference variables
        x.zeroize();
        y_s.zeroize();
        y_x.zeroize();
        y_r.zeroize();

        CiphertextCiphertextEqualityProof {
            Y_0,
            Y_1,
            Y_2,
            Y_3,
            z_s,
            z_x,
            z_r,
        }
    }

    /// Verifies a ciphertext-ciphertext equality proof.
    ///
    /// * `first_pubkey` - The ElGamal pubkey associated with the first ciphertext to be proved
    /// * `second_pubkey` - The ElGamal pubkey associated with the second ciphertext to be proved
    /// * `first_ciphertext` - The first ElGamal ciphertext to be proved
    /// * `second_ciphertext` - The second ElGamal ciphertext to be proved
    /// * `transcript` - The transcript that does the bookkeeping for the Fiat-Shamir heuristic
    pub fn verify(
        self,
        first_pubkey: &ElGamalPubkey,
        second_pubkey: &ElGamalPubkey,
        first_ciphertext: &ElGamalCiphertext,
        second_ciphertext: &ElGamalCiphertext,
        transcript: &mut Transcript,
    ) -> Result<(), EqualityProofVerificationError> {
        transcript.ciphertext_ciphertext_equality_proof_domain_separator();

        // extract the relevant scalar and Ristretto points from the inputs
        let P_first = first_pubkey.get_point();
        let C_first = first_ciphertext.commitment.get_point();
        let D_first = first_ciphertext.handle.get_point();

        let P_second = second_pubkey.get_point();
        let C_second = second_ciphertext.commitment.get_point();
        let D_second = second_ciphertext.handle.get_point();

        // include Y_0, Y_1, Y_2 to transcript and extract challenges
        transcript.validate_and_append_point(b"Y_0", &self.Y_0)?;
        transcript.validate_and_append_point(b"Y_1", &self.Y_1)?;
        transcript.validate_and_append_point(b"Y_2", &self.Y_2)?;
        transcript.validate_and_append_point(b"Y_3", &self.Y_3)?;

        let c = transcript.challenge_scalar(b"c");

        transcript.append_scalar(b"z_s", &self.z_s);
        transcript.append_scalar(b"z_x", &self.z_x);
        transcript.append_scalar(b"z_r", &self.z_r);
        let w = transcript.challenge_scalar(b"w"); // w used for batch verification
        let ww = &w * &w;
        let www = &w * &ww;

        let w_negated = -&w;
        let ww_negated = -&ww;
        let www_negated = -&www;

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
        let Y_3 = self
            .Y_3
            .decompress()
            .ok_or(SigmaProofVerificationError::Deserialization)?;

        let check = RistrettoPoint::vartime_multiscalar_mul(
            vec![
                &self.z_s,            // z_s
                &(-&c),               // -c
                &(-&Scalar::ONE),     // -identity
                &(&w * &self.z_x),    // w * z_x
                &(&w * &self.z_s),    // w * z_s
                &(&w_negated * &c),   // -w * c
                &w_negated,           // -w
                &(&ww * &self.z_x),   // ww * z_x
                &(&ww * &self.z_r),   // ww * z_r
                &(&ww_negated * &c),  // -ww * c
                &ww_negated,          // -ww
                &(&www * &self.z_r),  // www * z_r
                &(&www_negated * &c), // -www * c
                &www_negated,
            ],
            vec![
                P_first,  // P_first
                &(*H),    // H
                &Y_0,     // Y_0
                &G,       // G
                D_first,  // D_first
                C_first,  // C_first
                &Y_1,     // Y_1
                &G,       // G
                &(*H),    // H
                C_second, // C_second
                &Y_2,     // Y_2
                P_second, // P_second
                D_second, // D_second
                &Y_3,     // Y_3
            ],
        );

        if check.is_identity() {
            Ok(())
        } else {
            Err(SigmaProofVerificationError::AlgebraicRelation.into())
        }
    }

    pub fn to_bytes(&self) -> [u8; CIPHERTEXT_CIPHERTEXT_EQUALITY_PROOF_LEN] {
        let mut buf = [0_u8; CIPHERTEXT_CIPHERTEXT_EQUALITY_PROOF_LEN];
        let mut chunks = buf.chunks_mut(UNIT_LEN);

        chunks.next().unwrap().copy_from_slice(self.Y_0.as_bytes());
        chunks.next().unwrap().copy_from_slice(self.Y_1.as_bytes());
        chunks.next().unwrap().copy_from_slice(self.Y_2.as_bytes());
        chunks.next().unwrap().copy_from_slice(self.Y_3.as_bytes());
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
        let Y_3 = ristretto_point_from_optional_slice(chunks.next())?;
        let z_s = canonical_scalar_from_optional_slice(chunks.next())?;
        let z_x = canonical_scalar_from_optional_slice(chunks.next())?;
        let z_r = canonical_scalar_from_optional_slice(chunks.next())?;

        Ok(CiphertextCiphertextEqualityProof {
            Y_0,
            Y_1,
            Y_2,
            Y_3,
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
            encryption::pod::elgamal::{PodElGamalCiphertext, PodElGamalPubkey},
            sigma_proofs::pod::PodCiphertextCiphertextEqualityProof,
        },
        std::str::FromStr,
    };

    #[test]
    fn test_ciphertext_ciphertext_equality_proof_correctness() {
        // success case
        let first_keypair = ElGamalKeypair::new_rand();
        let second_keypair = ElGamalKeypair::new_rand();
        let message: u64 = 55;

        let first_ciphertext = first_keypair.pubkey().encrypt(message);

        let second_opening = PedersenOpening::new_rand();
        let second_ciphertext = second_keypair
            .pubkey()
            .encrypt_with(message, &second_opening);

        let mut prover_transcript = Transcript::new(b"Test");
        let mut verifier_transcript = Transcript::new(b"Test");

        let proof = CiphertextCiphertextEqualityProof::new(
            &first_keypair,
            second_keypair.pubkey(),
            &first_ciphertext,
            &second_opening,
            message,
            &mut prover_transcript,
        );

        proof
            .verify(
                first_keypair.pubkey(),
                second_keypair.pubkey(),
                &first_ciphertext,
                &second_ciphertext,
                &mut verifier_transcript,
            )
            .unwrap();

        // fail case: encrypted and committed messages are different
        let first_message: u64 = 55;
        let second_message: u64 = 77;

        let first_ciphertext = first_keypair.pubkey().encrypt(first_message);

        let second_opening = PedersenOpening::new_rand();
        let second_ciphertext = second_keypair
            .pubkey()
            .encrypt_with(second_message, &second_opening);

        let mut prover_transcript = Transcript::new(b"Test");
        let mut verifier_transcript = Transcript::new(b"Test");

        let proof = CiphertextCiphertextEqualityProof::new(
            &first_keypair,
            second_keypair.pubkey(),
            &first_ciphertext,
            &second_opening,
            message,
            &mut prover_transcript,
        );

        assert!(proof
            .verify(
                first_keypair.pubkey(),
                second_keypair.pubkey(),
                &first_ciphertext,
                &second_ciphertext,
                &mut verifier_transcript
            )
            .is_err());
    }

    #[test]
    fn test_ciphertext_ciphertext_equality_proof_string() {
        let first_pubkey_str = "VOPKaqo4nsX4XnbgGjCKHkLkR6JG1jX9D5G/e0EuYmM=";
        let pod_first_pubkey = PodElGamalPubkey::from_str(first_pubkey_str).unwrap();
        let first_pubkey: ElGamalPubkey = pod_first_pubkey.try_into().unwrap();

        let second_pubkey_str = "JnVhtKo9B7g9c8Obo/5/EqvA59i3TvtuOcQWf17T7SU=";
        let pod_second_pubkey = PodElGamalPubkey::from_str(second_pubkey_str).unwrap();
        let second_pubkey: ElGamalPubkey = pod_second_pubkey.try_into().unwrap();

        let first_ciphertext_str = "oKv6zxN051MXdk2cISD+CUsH2+FINoH1iB4WZyuy6nNkE7Q+eLiY9JB8itJhgKHJEA/1sAzDvpnRlLL06OXvIg==";
        let pod_first_ciphertext = PodElGamalCiphertext::from_str(first_ciphertext_str).unwrap();
        let first_ciphertext: ElGamalCiphertext = pod_first_ciphertext.try_into().unwrap();

        let second_ciphertext_str = "ooSA2cQDqutgyCBoMiQktM1Cu4NDNEbphF010gjG4iF0iMK1N+u/Qxqk0wwO/+w+5S6RiicwPs4mEKRJpFiHEw==";
        let pod_second_ciphertext = PodElGamalCiphertext::from_str(second_ciphertext_str).unwrap();
        let second_ciphertext: ElGamalCiphertext = pod_second_ciphertext.try_into().unwrap();

        let proof_str = "MlfRDO4sBPbpciEXci3QfVSLVABAJ0s8wMZ/Uz3AyETmGJ1BUE961fHIiNQXPD0j1uu1Josj//E8loPD1w+4E3bfDBJ3Mp2YqeOv41Bdec02YXlAotTGjq/UfncGdUhyampkuXUmSvnmkf5BIp4nr3X18cR9KHTAzBrKv6erjAxIckyRnACaZGEx+ZboEb3FBEXqTklytT1nrebbwkjvDUWbcpZrE+xxBWYek3qeq1x1debzxVhtS2yx44cvR5UIGLzGYa2ec/xh7wvyNEbnX80rZju2dztr4bN5f2vrTgk=";
        let pod_proof = PodCiphertextCiphertextEqualityProof::from_str(proof_str).unwrap();
        let proof: CiphertextCiphertextEqualityProof = pod_proof.try_into().unwrap();

        let mut verifier_transcript = Transcript::new(b"Test");

        proof
            .verify(
                &first_pubkey,
                &second_pubkey,
                &first_ciphertext,
                &second_ciphertext,
                &mut verifier_transcript,
            )
            .unwrap();
    }
}
