use crate::{
    loader::{
        halo2::{Halo2Loader, PoseidonTranscript},
        LoadedScalar, Loader,
    },
    scheme::kzg::{
        accumulation::{AccumulationStrategy, Accumulator},
        MSM,
    },
    util::{Domain, TranscriptRead},
    Error,
};
use ff::PrimeField;
use group::Curve;
use halo2_curves::{
    bn256::{Fq, Fr, G1Affine, G1},
    FieldExt,
};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::BufReader;
use std::marker::PhantomData;
use std::ops::Neg;

#[derive(Serialize, Deserialize, Debug)]
// #[serde(rename_all = "camelCase")]
struct VerificationKeyUnInit {
    protocol: String,
    curve: String,
    nPublic: usize,
    power: usize,
    k1: String,
    k2: String,
    Qm: Vec<String>,
    Ql: Vec<String>,
    Qr: Vec<String>,
    Qo: Vec<String>,
    Qc: Vec<String>,
    S1: Vec<String>,
    S2: Vec<String>,
    S3: Vec<String>,
}

pub struct Protocol<C: Curve> {
    domain: Domain<C::Scalar>,
    public_inputs_count: usize,
    k1: C::Scalar,
    k2: C::Scalar,
    Qm: C,
    Ql: C,
    Qr: C,
    Qo: C,
    Qc: C,
    S1: C,
    S2: C,
    S3: C,
}

pub fn str_to_g1(x: &str, y: &str, z: &str) -> G1 {
    let x = Fq::from_str_vartime(x).unwrap();
    let y = Fq::from_str_vartime(y).unwrap();
    let z = Fq::from_str_vartime(z).unwrap();
    G1 { x, y, z }
}

impl<C: Curve> Protocol<C> {
    pub fn read<B: PrimeField>(path: &str) {
        let file = File::open(path).unwrap();
        let reader = BufReader::new(file);
        let vk_unint: VerificationKeyUnInit = serde_json::from_reader(reader).unwrap();

        let k1 = C::Scalar::from_str_vartime(vk_unint.k1.as_str()).unwrap();
        let fw = B::from_str_vartime(vk_unint.Qc[0].as_str()).unwrap();
        let Qm: C = str_to_g1(
            vk_unint.Qc[0].as_str(),
            vk_unint.Qc[1].as_str(),
            vk_unint.Qc[2].as_str(),
        );

        // load group

        println!("{:#?}", Qm);

        // let p = Self {
        //     domain: Domain::new(vk_unint.power),
        //     public_inputs_count: vk_unint.nPublic,
        //     k1: C::Scalar::from_str_vartime(vk_unint.k1.as_str()).unwrap(),
        //     k2: C::Scalar::from_str_vartime(vk_unint.k1.as_str()).unwrap(),
        // };
    }
}

pub struct Challenges<C: Curve, L: Loader<C>> {
    beta: L::LoadedScalar,
    alpha: L::LoadedScalar,
    gamma: L::LoadedScalar,
    xi: L::LoadedScalar,
    v: L::LoadedScalar,
    u: L::LoadedScalar,
}

pub struct CircomPlonkProof<C: Curve, L: Loader<C>> {
    A: L::LoadedEcPoint,
    B: L::LoadedEcPoint,
    C: L::LoadedEcPoint,
    Z: L::LoadedEcPoint,
    T1: L::LoadedEcPoint,
    T2: L::LoadedEcPoint,
    T3: L::LoadedEcPoint,
    Wxi: L::LoadedEcPoint,
    Wxiw: L::LoadedEcPoint,
    eval_a: L::LoadedScalar,
    eval_b: L::LoadedScalar,
    eval_c: L::LoadedScalar,
    eval_s1: L::LoadedScalar,
    eval_s2: L::LoadedScalar,
    eval_zw: L::LoadedScalar,
    eval_r: L::LoadedScalar,
    challenges: Challenges<C, L>,
}

impl<C: Curve, L: Loader<C>> CircomPlonkProof<C, L> {
    fn read<T: TranscriptRead<C, L>>(
        public_signals: &Vec<L::LoadedScalar>,
        transcript: &mut T,
    ) -> Result<Self, Error> {
        public_signals.iter().for_each(|signal| {
            transcript.common_scalar(signal).unwrap();
        });

        let A = transcript.read_ec_point()?;
        let B = transcript.read_ec_point()?;
        let C = transcript.read_ec_point()?;

        let beta = transcript.squeeze_challenge();

        transcript.common_scalar(&beta)?;
        let gamma = transcript.squeeze_challenge();

        let Z = transcript.read_ec_point()?;
        let alpha = transcript.squeeze_challenge();

        let T1 = transcript.read_ec_point()?;
        let T2 = transcript.read_ec_point()?;
        let T3 = transcript.read_ec_point()?;
        let xi = transcript.squeeze_challenge();

        let eval_points = transcript.read_n_scalars(7)?;
        // let eval_a = eval_points[0];
        // let eval_b = eval_points[1];
        // let eval_c = eval_points[2];
        // let eval_s1 = eval_points[3];
        // let eval_s2 = eval_points[4];
        // let eval_zw = eval_points[5];
        // let eval_r = eval_points[6];

        let v = transcript.squeeze_challenge();

        let Wxi = transcript.read_ec_point()?;
        let Wxiw = transcript.read_ec_point()?;
        let u = transcript.squeeze_challenge();

        Ok(Self {
            A,
            B,
            C,
            Z,
            T1,
            T2,
            T3,
            Wxi,
            Wxiw,
            eval_a: eval_points[0].clone(),
            eval_b: eval_points[1].clone(),
            eval_c: eval_points[2].clone(),
            eval_s1: eval_points[3].clone(),
            eval_s2: eval_points[4].clone(),
            eval_zw: eval_points[5].clone(),
            eval_r: eval_points[6].clone(),
            challenges: Challenges {
                beta,
                alpha,
                gamma,
                xi,
                v,
                u,
            },
        })
    }
}

#[derive(Default)]
pub struct CircomPlonkAccumulationScheme<C, L, T, S> {
    _marker_c: PhantomData<C>,
    _marker_l: PhantomData<L>,
    _marker_t: PhantomData<T>,
    _marker_s: PhantomData<S>,
}

impl<C, L, T, S> CircomPlonkAccumulationScheme<C, L, T, S>
where
    C: Curve,
    L: Loader<C>,
    T: TranscriptRead<C, L>,
    S: AccumulationStrategy<C, L, T, CircomPlonkProof<C, L>>,
{
    fn accumulate(
        protocol: &Protocol<C>,
        loader: &L,
        public_signals: &Vec<L::LoadedScalar>,
        transcript: &mut T,
        strategy: &mut S,
    ) -> Result<S::Output, crate::Error> {
        // perform necessary checks
        assert_eq!(public_signals.len(), protocol.public_inputs_count);

        let proof = CircomPlonkProof::read(public_signals, transcript)?;

        let Qm = loader.ec_point_load_const(&protocol.Qm);
        let Ql = loader.ec_point_load_const(&protocol.Ql);
        let Qr = loader.ec_point_load_const(&protocol.Qr);
        let Qo = loader.ec_point_load_const(&protocol.Qo);
        let Qc = loader.ec_point_load_const(&protocol.Qc);
        let S1 = loader.ec_point_load_const(&protocol.S1);
        let S2 = loader.ec_point_load_const(&protocol.S2);
        let S3 = loader.ec_point_load_const(&protocol.S3);
        let k1 = loader.load_const(&protocol.k1);
        let k2 = loader.load_const(&protocol.k2);

        let xi = proof.challenges.xi.clone();
        let n = loader.load_const(&C::Scalar::from(protocol.domain.n as u64));
        let n_inv = loader.load_const(&protocol.domain.n_inv);
        let xi_power_n = xi.clone().pow_const(protocol.domain.n as u64);
        let omega = loader.load_const(&protocol.domain.gen);
        let omega_inv = loader.load_const(&protocol.domain.gen_inv);
        let omega_inv_powers = omega_inv.clone().powers(public_signals.len());

        // z_h(xi) = xi^n - 1;
        let one = loader.load_one();
        let z_h_eval_xi = xi_power_n.clone() - one.clone();

        // Compute first lagrange evaluation.
        // Snarkjs's plonk prover starts with `omega^0`
        // in permutation polynomial. Thus we compute
        // `L0(xi)` here.
        //
        // `L0(xi) = (xi^n) - 1 / (n * (xi - 1))`
        //
        // More info on this - https://github.com/ZK-Garage/plonk/blob/79dffa1bacbe73ab42e2d7e48194efe5c0070bd6/plonk-core/src/proof_system/proof.rs#L622
        let l1_eval_xi = {
            let denom = xi.clone() - one.clone();
            z_h_eval_xi.clone() * denom.invert().unwrap()
        };

        // Compute public input poly evaluation at `xi`.
        // We do this using `barycentric evaluation` approach.
        // For more details on this approach check following:
        //  (1) https://hackmd.io/@vbuterin/barycentric_evaluation
        //  (2) https://github.com/ZK-Garage/plonk/blob/79dffa1bacbe73ab42e2d7e48194efe5c0070bd6/plonk-core/src/proof_system/proof.rs#L635
        //
        // TODO: We store `omegas` in `vk`. We only need them at this
        // step of verification. This means we shall only load omages
        // omegas_inv for range (0..public_inputs.length). Implement this
        // optimization.
        let pi_poly_eval_xi = {
            // (xi^n - 1) / n
            //
            // TODO: store `n.invert()` in `vk` to avoid
            // having to constrain it in every accumulation step.
            let numerator = z_h_eval_xi.clone() * n_inv.clone();

            // In case of no public inputs PI(x)
            // can be reduced to
            // PI(x) = (x^n - 1) / n
            if public_signals.len() == 0 {
                numerator
            } else {
                let denominator = {
                    let denoms: Vec<L::LoadedScalar> = (0..public_signals.len())
                        .map(|index| {
                            // (xi - omega^j) * omega^-j => (omega^-j * xi - 1)
                            // for `j`th index.
                            let value =
                                (xi.clone() * omega_inv_powers[index].clone()) - one.clone();
                            value.invert().unwrap_or_else(|| value.clone())
                        })
                        .collect();

                    // Computes
                    // `sum_of { pi_j * (xi * omega^-j - 1)^-1 }`
                    // for j in range 0..public_signals.len()
                    let mut sum = denoms[0].clone() * public_signals[0].clone();
                    denoms.iter().enumerate().for_each(|(index, d)| {
                        if index > 1 {
                            sum = d.clone() * public_signals[index].clone();
                        }
                    });
                    sum
                };
                numerator * denominator
            }
        };

        // Compute pairing rhs
        let rhs = {
            let mut rhs = MSM::default();

            let ab = proof.eval_a.clone() * proof.eval_b.clone();
            rhs.push(ab, Qm);
            rhs.push(proof.eval_a.clone(), Ql);
            rhs.push(proof.eval_b.clone(), Qr);
            rhs.push(proof.eval_c.clone(), Qo);
            rhs.push(one.clone(), Qc);

            let alpha = proof.challenges.alpha.clone();
            let alpha_square = proof.challenges.alpha.clone().square();

            let scalar_batch_poly_commit_identity = {
                let a = proof.eval_a.clone()
                    + (proof.challenges.beta.clone() * proof.challenges.xi.clone())
                    + proof.challenges.gamma.clone();
                let b = proof.eval_b.clone()
                    + (proof.challenges.beta.clone() * k1.clone() * proof.challenges.xi.clone())
                    + proof.challenges.gamma.clone();
                let c = proof.eval_b.clone()
                    + (proof.challenges.beta.clone() * k2.clone() * proof.challenges.xi.clone())
                    + proof.challenges.gamma.clone();
                let val = a * b * c * alpha.clone();
                let val2 = l1_eval_xi.clone() * alpha_square.clone() + proof.challenges.u.clone();
                val + val2
            };
            rhs.push(scalar_batch_poly_commit_identity, proof.Z.clone());

            let scalar_batch_poly_commit_permuted = {
                let a = proof.eval_a.clone()
                    + (proof.challenges.beta.clone() * proof.eval_s1.clone())
                    + proof.challenges.gamma.clone();
                let b = proof.eval_b.clone()
                    + (proof.challenges.beta.clone() * proof.eval_s2.clone())
                    + proof.challenges.gamma.clone();
                a * b * alpha.clone() * proof.eval_zw.clone()
            };
            rhs.push(scalar_batch_poly_commit_permuted.neg(), S3);

            let neg_z_h_eval_xi = z_h_eval_xi.clone().neg();
            let neg_z_h_eval_xi_by_xi = neg_z_h_eval_xi.clone() * xi_power_n.clone();
            let neg_z_h_eval_xi_by_xi_2n = neg_z_h_eval_xi.clone() * xi_power_n.clone().square();
            rhs.push(neg_z_h_eval_xi.clone(), proof.T1.clone());
            rhs.push(neg_z_h_eval_xi_by_xi, proof.T2.clone());
            rhs.push(neg_z_h_eval_xi_by_xi_2n, proof.T3.clone());

            // powers of `v`
            let v_powers = proof.challenges.v.powers(6);

            rhs.push(v_powers[1].clone(), proof.A.clone());
            rhs.push(v_powers[2].clone(), proof.B.clone());
            rhs.push(v_powers[3].clone(), proof.C.clone());
            rhs.push(v_powers[4].clone(), S1);
            rhs.push(v_powers[5].clone(), S2);

            let r0 = {
                let l1_alpha_sq = l1_eval_xi * alpha_square;

                // permutation product
                let p1 = proof.eval_a.clone()
                    + (proof.challenges.beta.clone() * proof.eval_s1.clone())
                    + proof.challenges.gamma.clone();
                let p2 = proof.eval_b.clone()
                    + (proof.challenges.beta.clone() * proof.eval_s2.clone())
                    + proof.challenges.gamma.clone();
                let p3 =
                    (proof.eval_c.clone() + proof.challenges.gamma.clone()) * proof.eval_zw.clone();
                let pp = p1 * p2 * p3 * alpha.clone();

                pi_poly_eval_xi - l1_alpha_sq - pp
            };

            let group_batch_eval_scalar = {
                let mut sum = r0.neg() + (v_powers[1].clone() * proof.eval_a.clone());
                sum = sum + (v_powers[2].clone() * proof.eval_b.clone());
                sum = sum + (v_powers[3].clone() * proof.eval_c.clone());
                sum = sum + (v_powers[4].clone() * proof.eval_s1.clone());
                sum = sum + (v_powers[5].clone() * proof.eval_s2.clone());
                sum = sum + (proof.challenges.u.clone() * proof.eval_zw.clone());
                sum.neg()
            };
            rhs.push(group_batch_eval_scalar.neg(), loader.ec_point_load_one());

            let u_xi_omega = proof.challenges.u.clone() * xi.clone() * omega.clone();
            rhs.push(xi.clone(), proof.Wxi.clone());
            rhs.push(u_xi_omega.clone(), proof.Wxi.clone());

            rhs
        };

        // Compute pairing lhs
        let mut lhs = MSM::default();
        lhs.push(one.clone(), proof.Wxi.clone());
        lhs.push(proof.challenges.u.clone(), proof.Wxiw.clone());

        let accumulator = Accumulator::new(lhs, rhs);
        strategy.process(loader, transcript, proof, accumulator)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read() {
        let cwd = std::env::current_dir().unwrap();
        let cwd = cwd.to_str().unwrap();
        let f =
            Protocol::<G1>::read::<Fq>(format!("{}/target/verification_key.json", cwd).as_str());
    }
}
