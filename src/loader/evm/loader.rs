use crate::{
    loader::evm::{
        code::{Code, Precompiled},
        modulus,
    },
    loader::{EcPointLoader, LoadedEcPoint, LoadedScalar, Loader, ScalarLoader},
    util::{Curve, FieldOps, PrimeField, UncompressedEncoding},
};
use ethereum_types::{U256, U512};
use std::{
    cell::RefCell,
    fmt::{self, Debug},
    iter,
    ops::{Add, AddAssign, Mul, MulAssign, Neg, Sub, SubAssign},
    rc::Rc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Value<T> {
    Constant(T),
    Memory(usize),
}

#[derive(Clone, Debug)]
pub struct EvmLoader {
    base_modulus: U256,
    scalar_modulus: U256,
    code: RefCell<Code>,
    ptr: RefCell<usize>,
    #[cfg(test)]
    gas_metering_ids: RefCell<Vec<String>>,
}

impl EvmLoader {
    pub fn new<Base, Scalar>() -> Rc<Self>
    where
        Base: PrimeField<Repr = [u8; 32]>,
        Scalar: PrimeField<Repr = [u8; 32]>,
    {
        let base_modulus = modulus::<Base>();
        let scalar_modulus = modulus::<Scalar>();
        let code = Code::new([1.into(), base_modulus, scalar_modulus - 1, scalar_modulus])
            .push(1)
            .to_owned();
        Rc::new(Self {
            base_modulus,
            scalar_modulus,
            code: RefCell::new(code),
            ptr: RefCell::new(0),
            #[cfg(test)]
            gas_metering_ids: RefCell::new(Vec::new()),
        })
    }

    pub fn code(self: &Rc<Self>) -> Vec<u8> {
        let mut code = self.code.borrow().clone();
        let dst = code.len() + 9;
        code.push(dst)
            .jumpi()
            .push(0)
            .push(0)
            .revert()
            .jumpdest()
            .stop()
            .to_owned()
            .into()
    }

    pub fn allocate(self: &Rc<Self>, size: usize) -> usize {
        let ptr = *self.ptr.borrow();
        *self.ptr.borrow_mut() += size;
        ptr
    }

    fn scalar(self: &Rc<Self>, value: Value<U256>) -> Scalar {
        Scalar {
            loader: self.clone(),
            value,
        }
    }

    fn ec_point(self: &Rc<Self>, value: Value<(U256, U256)>) -> EcPoint {
        EcPoint {
            loader: self.clone(),
            value,
        }
    }

    fn push(self: &Rc<Self>, scalar: &Scalar) {
        match scalar.value {
            Value::Constant(constant) => {
                self.code.borrow_mut().push(constant);
            }
            Value::Memory(ptr) => {
                self.code.borrow_mut().push(ptr).mload();
            }
        }
    }

    pub fn calldataload_scalar(self: &Rc<Self>, offset: usize) -> Scalar {
        let ptr = self.allocate(0x20);
        self.code
            .borrow_mut()
            .push(self.scalar_modulus)
            .push(offset)
            .calldataload()
            .r#mod()
            .push(ptr)
            .mstore();
        self.scalar(Value::Memory(ptr))
    }

    pub fn calldataload_ec_point(self: &Rc<Self>, offset: usize) -> EcPoint {
        let ptr = self.allocate(0x40);
        self.code
            .borrow_mut()
            // [..., success]
            .push(offset)
            // [..., success, x_cd_ptr]
            .calldataload()
            // [..., success, x]
            .dup(0)
            // [..., success, x, x]
            .push(ptr)
            // [..., success, x, x, x_ptr]
            .mstore()
            // [..., success, x]
            .push(offset + 0x20)
            // [..., success, x, y_cd_ptr]
            .calldataload()
            // [..., success, x, y]
            .dup(0)
            // [..., success, x, y, y]
            .push(ptr + 0x20)
            // [..., success, x, y, y, y_ptr]
            .mstore();
        // [..., success, x, y]
        self.validate_ec_point();
        self.ec_point(Value::Memory(ptr))
    }

    pub fn calldataload_ec_point_from_limbs<const LIMBS: usize, const BITS: usize>(
        self: &Rc<Self>,
        offset: usize,
    ) -> EcPoint {
        let ptr = self.allocate(0x40);
        for (ptr, offset) in [(ptr, offset), (ptr + 0x20, offset + LIMBS * 0x20)] {
            for idx in 0..LIMBS {
                if idx == 0 {
                    self.code
                        .borrow_mut()
                        // [..., success]
                        .push(offset)
                        // [..., success, x_limb_0_ptr]
                        .calldataload();
                    // [..., success, x_limb_0]
                } else {
                    self.code
                        .borrow_mut()
                        // [..., success, x_acc]
                        .push(offset + idx * 0x20)
                        // [..., success, x_acc, x_limb_i_ptr]
                        .calldataload()
                        // [..., success, x_acc, x_limb_i]
                        .push(idx * BITS)
                        // [..., success, x_acc, x_limb_i, shift]
                        .shl()
                        // [..., success, x_acc, x_limb_i << shift]
                        .add();
                    // [..., success, x_acc]
                }
            }
            self.code
                .borrow_mut()
                // [..., success, x]
                .dup(0)
                // [..., success, x, x]
                .push(ptr)
                // [..., success, x, x, x_ptr]
                .mstore();
            // [..., success, x]
        }
        // [..., success, x, y]
        self.validate_ec_point();
        self.ec_point(Value::Memory(ptr))
    }

    fn validate_ec_point(self: &Rc<Self>) {
        self.code
            .borrow_mut()
            // [..., success, x, y]
            .push(self.base_modulus)
            // [..., success, x, y, p]
            .dup(2)
            // [..., success, x, y, p, x]
            .lt()
            // [..., success, x, y, x_lt_p]
            .push(self.base_modulus)
            // [..., success, x, y, x_lt_p, p]
            .dup(2)
            // [..., success, x, y, x_lt_p, p, y]
            .lt()
            // [..., success, x, y, x_lt_p, y_lt_p]
            .and()
            // [..., success, x, y, valid]
            .dup(2)
            // [..., success, x, y, valid, x]
            .iszero()
            // [..., success, x, y, valid, x_is_zero]
            .dup(2)
            // [..., success, x, y, valid, x_is_zero, y]
            .iszero()
            // [..., success, x, y, valid, x_is_zero, y_is_zero]
            .or()
            // [..., success, x, y, valid, x_or_y_is_zero]
            .not()
            // [..., success, x, y, valid, x_and_y_is_not_zero]
            .and()
            // [..., success, x, y, valid]
            .push(self.base_modulus)
            // [..., success, x, y, valid, p]
            .dup(2)
            // [..., success, x, y, valid, p, y]
            .dup(0)
            // [..., success, x, y, valid, p, y, y]
            .mulmod()
            // [..., success, x, y, valid, y_square]
            .push(self.base_modulus)
            // [..., success, x, y, valid, y_square, p]
            .push(3)
            // [..., success, x, y, valid, y_square, p, 3]
            .push(self.base_modulus)
            // [..., success, x, y, valid, y_square, p, 3, p]
            .dup(6)
            // [..., success, x, y, valid, y_square, p, 3, p, x]
            .push(self.base_modulus)
            // [..., success, x, y, valid, y_square, p, 3, p, x, p]
            .dup(1)
            // [..., success, x, y, valid, y_square, p, 3, p, x, p, x]
            .dup(0)
            // [..., success, x, y, valid, y_square, p, 3, p, x, p, x, x]
            .mulmod()
            // [..., success, x, y, valid, y_square, p, 3, p, x, x_square]
            .mulmod()
            // [..., success, x, y, valid, y_square, p, 3, x_cube]
            .addmod()
            // [..., success, x, y, valid, y_square, x_cube_plus_3]
            .eq()
            // [..., success, x, y, valid, y_square_eq_x_cube_plus_3]
            .and()
            // [..., success, x, y, valid]
            .swap(2)
            // [..., success, valid, y, x]
            .pop()
            // [..., success, valid, y]
            .pop()
            // [..., success, valid]
            .and();
    }

    pub fn squeeze_challenge(self: &Rc<Self>, ptr: usize, len: usize) -> (usize, Scalar) {
        assert!(len > 0 && len % 0x20 == 0);

        let (ptr, len) = if len == 0x20 {
            let ptr = if ptr + len != *self.ptr.borrow() {
                (ptr..ptr + len)
                    .step_by(0x20)
                    .map(|ptr| self.dup_scalar(&self.scalar(Value::Memory(ptr))))
                    .collect::<Vec<_>>()
                    .first()
                    .unwrap()
                    .ptr()
            } else {
                ptr
            };
            self.code.borrow_mut().push(1).push(ptr + 0x20).mstore8();
            (ptr, len + 1)
        } else {
            (ptr, len)
        };

        let challenge_ptr = self.allocate(0x20);
        let hash_ptr = self.allocate(0x20);

        self.code
            .borrow_mut()
            .push(self.scalar_modulus)
            .push(len)
            .push(ptr)
            .keccak256()
            .dup(0)
            .push(hash_ptr)
            .mstore()
            .r#mod()
            .push(challenge_ptr)
            .mstore();

        (hash_ptr, self.scalar(Value::Memory(challenge_ptr)))
    }

    pub fn copy_scalar(self: &Rc<Self>, scalar: &Scalar, ptr: usize) {
        match scalar.value {
            Value::Constant(constant) => {
                self.code.borrow_mut().push(constant).push(ptr).mstore();
            }
            Value::Memory(src_ptr) => {
                self.code
                    .borrow_mut()
                    .push(src_ptr)
                    .mload()
                    .push(ptr)
                    .mstore();
            }
        }
    }

    pub fn dup_scalar(self: &Rc<Self>, scalar: &Scalar) -> Scalar {
        let ptr = self.allocate(0x20);
        self.copy_scalar(scalar, ptr);
        self.scalar(Value::Memory(ptr))
    }

    fn dup_ec_point(self: &Rc<Self>, value: &EcPoint) -> EcPoint {
        let ptr = self.allocate(0x40);
        match value.value {
            Value::Constant((x, y)) => {
                self.code
                    .borrow_mut()
                    .push(x)
                    .push(ptr)
                    .mstore()
                    .push(y)
                    .push(ptr + 0x20)
                    .mstore();
            }
            Value::Memory(src_ptr) => {
                self.code
                    .borrow_mut()
                    .push(src_ptr)
                    .mload()
                    .push(ptr)
                    .mstore()
                    .push(src_ptr + 0x20)
                    .mload()
                    .push(ptr + 0x20)
                    .mstore();
            }
        }
        self.ec_point(Value::Memory(ptr))
    }

    fn staticcall(self: &Rc<Self>, precompile: Precompiled, cd_ptr: usize, rd_ptr: usize) {
        let (cd_len, rd_len) = match precompile {
            Precompiled::BigModExp => (0xc0, 0x20),
            Precompiled::Bn254Add => (0x80, 0x40),
            Precompiled::Bn254ScalarMul => (0x60, 0x40),
            Precompiled::Bn254Pairing => (0x180, 0x20),
        };
        self.code
            .borrow_mut()
            .push(rd_len)
            .push(rd_ptr)
            .push(cd_len)
            .push(cd_ptr)
            .push(precompile as usize)
            .gas()
            .staticcall()
            .and();
    }

    fn invert(self: &Rc<Self>, scalar: &Scalar) -> Scalar {
        let rd_ptr = self.allocate(0x20);
        let [cd_ptr, ..] = [
            &self.scalar(Value::Constant(0x20.into())),
            &self.scalar(Value::Constant(0x20.into())),
            &self.scalar(Value::Constant(0x20.into())),
            scalar,
            &self.scalar(Value::Constant(self.scalar_modulus - 2)),
            &self.scalar(Value::Constant(self.scalar_modulus)),
        ]
        .map(|value| self.dup_scalar(value).ptr());
        self.staticcall(Precompiled::BigModExp, cd_ptr, rd_ptr);
        self.scalar(Value::Memory(rd_ptr))
    }

    fn ec_point_add(self: &Rc<Self>, lhs: &EcPoint, rhs: &EcPoint) -> EcPoint {
        let rd_ptr = self.dup_ec_point(lhs).ptr();
        self.dup_ec_point(rhs);
        self.staticcall(Precompiled::Bn254Add, rd_ptr, rd_ptr);
        self.ec_point(Value::Memory(rd_ptr))
    }

    fn ec_point_sub(self: &Rc<Self>, _: &EcPoint, _: &EcPoint) -> EcPoint {
        unreachable!()
    }

    fn ec_point_neg(self: &Rc<Self>, _: &EcPoint) -> EcPoint {
        unreachable!()
    }

    fn ec_point_scalar_mul(self: &Rc<Self>, ec_point: &EcPoint, scalar: &Scalar) -> EcPoint {
        let rd_ptr = self.dup_ec_point(ec_point).ptr();
        self.dup_scalar(scalar);
        self.staticcall(Precompiled::Bn254ScalarMul, rd_ptr, rd_ptr);
        self.ec_point(Value::Memory(rd_ptr))
    }

    pub fn pairing(
        self: &Rc<Self>,
        lhs: &EcPoint,
        g2: (U256, U256, U256, U256),
        rhs: &EcPoint,
        minus_s_g2: (U256, U256, U256, U256),
    ) {
        let rd_ptr = self.dup_ec_point(lhs).ptr();
        self.allocate(0x80);
        self.code
            .borrow_mut()
            .push(g2.0)
            .push(rd_ptr + 0x40)
            .mstore()
            .push(g2.1)
            .push(rd_ptr + 0x60)
            .mstore()
            .push(g2.2)
            .push(rd_ptr + 0x80)
            .mstore()
            .push(g2.3)
            .push(rd_ptr + 0xa0)
            .mstore();
        self.dup_ec_point(rhs);
        self.allocate(0x80);
        self.code
            .borrow_mut()
            .push(minus_s_g2.0)
            .push(rd_ptr + 0x100)
            .mstore()
            .push(minus_s_g2.1)
            .push(rd_ptr + 0x120)
            .mstore()
            .push(minus_s_g2.2)
            .push(rd_ptr + 0x140)
            .mstore()
            .push(minus_s_g2.3)
            .push(rd_ptr + 0x160)
            .mstore();
        self.staticcall(Precompiled::Bn254Pairing, rd_ptr, rd_ptr);
        self.code.borrow_mut().push(rd_ptr).mload().and();
    }

    fn add(self: &Rc<Self>, lhs: &Scalar, rhs: &Scalar) -> Scalar {
        if let (Value::Constant(lhs), Value::Constant(rhs)) = (lhs.value, rhs.value) {
            let out = (U512::from(lhs) + U512::from(rhs)) % U512::from(self.scalar_modulus);
            return self.scalar(Value::Constant(out.try_into().unwrap()));
        }

        let ptr = self.allocate(0x20);

        self.code.borrow_mut().push(self.scalar_modulus);
        self.push(rhs);
        self.push(lhs);
        self.code.borrow_mut().addmod().push(ptr).mstore();

        self.scalar(Value::Memory(ptr))
    }

    fn sub(self: &Rc<Self>, lhs: &Scalar, rhs: &Scalar) -> Scalar {
        if rhs.is_const() {
            return self.add(lhs, &self.neg(rhs));
        }

        let ptr = self.allocate(0x20);

        self.code.borrow_mut().push(self.scalar_modulus);
        self.push(rhs);
        self.code.borrow_mut().push(self.scalar_modulus).sub();
        self.push(lhs);
        self.code.borrow_mut().addmod().push(ptr).mstore();

        self.scalar(Value::Memory(ptr))
    }

    fn mul(self: &Rc<Self>, lhs: &Scalar, rhs: &Scalar) -> Scalar {
        if let (Value::Constant(lhs), Value::Constant(rhs)) = (lhs.value, rhs.value) {
            let out = (U512::from(lhs) * U512::from(rhs)) % U512::from(self.scalar_modulus);
            return self.scalar(Value::Constant(out.try_into().unwrap()));
        }

        let ptr = self.allocate(0x20);

        self.code.borrow_mut().push(self.scalar_modulus);
        self.push(rhs);
        self.push(lhs);
        self.code.borrow_mut().mulmod().push(ptr).mstore();

        self.scalar(Value::Memory(ptr))
    }

    fn neg(self: &Rc<Self>, scalar: &Scalar) -> Scalar {
        if let Value::Constant(constant) = scalar.value {
            return self.scalar(Value::Constant(self.scalar_modulus - constant));
        }

        let ptr = self.allocate(0x20);

        self.push(scalar);
        self.code
            .borrow_mut()
            .push(self.scalar_modulus)
            .sub()
            .push(ptr)
            .mstore();

        self.scalar(Value::Memory(ptr))
    }
}

#[cfg(test)]
impl EvmLoader {
    fn start_gas_metering(self: &Rc<Self>, identifier: &str) {
        self.gas_metering_ids
            .borrow_mut()
            .push(identifier.to_string());
        self.code.borrow_mut().gas().swap(1);
    }

    fn end_gas_metering(self: &Rc<Self>) {
        self.code
            .borrow_mut()
            .swap(1)
            .push(9)
            .gas()
            .swap(2)
            .sub()
            .sub()
            .push(0)
            .push(0)
            .log1();
    }

    pub fn print_gas_metering(self: &Rc<Self>, costs: Vec<u64>) {
        for (identifier, cost) in self.gas_metering_ids.borrow().iter().zip(costs) {
            println!("{}: {}", identifier, cost);
        }
    }
}

#[derive(Clone)]
pub struct EcPoint {
    loader: Rc<EvmLoader>,
    value: Value<(U256, U256)>,
}

impl EcPoint {
    pub(super) fn loader(&self) -> &Rc<EvmLoader> {
        &self.loader
    }

    pub fn value(&self) -> Value<(U256, U256)> {
        self.value
    }

    pub fn is_const(&self) -> bool {
        matches!(self.value, Value::Constant(_))
    }

    pub fn ptr(&self) -> usize {
        match self.value {
            Value::Memory(ptr) => ptr,
            _ => unreachable!(),
        }
    }
}

impl Debug for EcPoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EcPoint")
            .field("value", &self.value)
            .finish()
    }
}

impl Add for EcPoint {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        self.loader.ec_point_add(&self, &rhs)
    }
}

impl Sub for EcPoint {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self {
        self.loader.ec_point_sub(&self, &rhs)
    }
}

impl Neg for EcPoint {
    type Output = Self;

    fn neg(self) -> Self {
        self.loader.ec_point_neg(&self)
    }
}

impl<'a> Add<&'a Self> for EcPoint {
    type Output = Self;

    fn add(self, rhs: &'a Self) -> Self {
        self.loader.ec_point_add(&self, rhs)
    }
}

impl<'a> Sub<&'a Self> for EcPoint {
    type Output = Self;

    fn sub(self, rhs: &'a Self) -> Self {
        self.loader.ec_point_sub(&self, rhs)
    }
}

impl AddAssign for EcPoint {
    fn add_assign(&mut self, rhs: Self) {
        *self = self.loader.ec_point_add(self, &rhs);
    }
}

impl SubAssign for EcPoint {
    fn sub_assign(&mut self, rhs: Self) {
        *self = self.loader.ec_point_sub(self, &rhs);
    }
}

impl<'a> AddAssign<&'a Self> for EcPoint {
    fn add_assign(&mut self, rhs: &'a Self) {
        *self = self.loader.ec_point_add(self, rhs);
    }
}

impl<'a> SubAssign<&'a Self> for EcPoint {
    fn sub_assign(&mut self, rhs: &'a Self) {
        *self = self.loader.ec_point_sub(self, rhs);
    }
}

impl PartialEq for EcPoint {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl<C> LoadedEcPoint<C> for EcPoint
where
    C: Curve + UncompressedEncoding<Uncompressed = [u8; 0x40]>,
    C::Scalar: PrimeField<Repr = [u8; 0x20]>,
{
    type Loader = Rc<EvmLoader>;

    fn loader(&self) -> &Rc<EvmLoader> {
        &self.loader
    }

    fn multi_scalar_multiplication(pairs: impl IntoIterator<Item = (Scalar, EcPoint)>) -> Self {
        pairs
            .into_iter()
            .map(|(scalar, ec_point)| match scalar.value {
                Value::Constant(constant) if constant == U256::one() => ec_point,
                _ => ec_point.loader.ec_point_scalar_mul(&ec_point, &scalar),
            })
            .reduce(|acc, ec_point| acc + ec_point)
            .unwrap()
    }
}

#[derive(Clone)]
pub struct Scalar {
    loader: Rc<EvmLoader>,
    value: Value<U256>,
}

impl Scalar {
    pub fn value(&self) -> Value<U256> {
        self.value
    }

    pub fn is_const(&self) -> bool {
        matches!(self.value, Value::Constant(_))
    }

    pub fn ptr(&self) -> usize {
        match self.value {
            Value::Memory(ptr) => ptr,
            _ => unreachable!(),
        }
    }
}

impl Debug for Scalar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Scalar")
            .field("value", &self.value)
            .finish()
    }
}

impl Add for Scalar {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        self.loader.add(&self, &rhs)
    }
}

impl Sub for Scalar {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self {
        self.loader.sub(&self, &rhs)
    }
}

impl Mul for Scalar {
    type Output = Self;

    fn mul(self, rhs: Self) -> Self {
        self.loader.mul(&self, &rhs)
    }
}

impl Neg for Scalar {
    type Output = Self;

    fn neg(self) -> Self {
        self.loader.neg(&self)
    }
}

impl<'a> Add<&'a Self> for Scalar {
    type Output = Self;

    fn add(self, rhs: &'a Self) -> Self {
        self.loader.add(&self, rhs)
    }
}

impl<'a> Sub<&'a Self> for Scalar {
    type Output = Self;

    fn sub(self, rhs: &'a Self) -> Self {
        self.loader.sub(&self, rhs)
    }
}

impl<'a> Mul<&'a Self> for Scalar {
    type Output = Self;

    fn mul(self, rhs: &'a Self) -> Self {
        self.loader.mul(&self, rhs)
    }
}

impl AddAssign for Scalar {
    fn add_assign(&mut self, rhs: Self) {
        *self = self.loader.add(self, &rhs);
    }
}

impl SubAssign for Scalar {
    fn sub_assign(&mut self, rhs: Self) {
        *self = self.loader.sub(self, &rhs);
    }
}

impl MulAssign for Scalar {
    fn mul_assign(&mut self, rhs: Self) {
        *self = self.loader.mul(self, &rhs);
    }
}

impl<'a> AddAssign<&'a Self> for Scalar {
    fn add_assign(&mut self, rhs: &'a Self) {
        *self = self.loader.add(self, rhs);
    }
}

impl<'a> SubAssign<&'a Self> for Scalar {
    fn sub_assign(&mut self, rhs: &'a Self) {
        *self = self.loader.sub(self, rhs);
    }
}

impl<'a> MulAssign<&'a Self> for Scalar {
    fn mul_assign(&mut self, rhs: &'a Self) {
        *self = self.loader.mul(self, rhs);
    }
}

impl FieldOps for Scalar {
    fn invert(&self) -> Option<Scalar> {
        Some(self.loader.invert(self))
    }
}

impl PartialEq for Scalar {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl<F: PrimeField<Repr = [u8; 0x20]>> LoadedScalar<F> for Scalar {
    type Loader = Rc<EvmLoader>;

    fn loader(&self) -> &Rc<EvmLoader> {
        &self.loader
    }

    fn batch_invert<'a>(values: impl IntoIterator<Item = &'a mut Self>) {
        let values = values.into_iter().collect::<Vec<_>>();
        let loader = &values.first().unwrap().loader;
        let products = iter::once(values[0].clone())
            .chain(
                iter::repeat_with(|| loader.allocate(0x20))
                    .map(|ptr| loader.scalar(Value::Memory(ptr)))
                    .take(values.len() - 1),
            )
            .collect::<Vec<_>>();

        loader.code.borrow_mut().push(loader.scalar_modulus);
        for _ in 2..values.len() {
            loader.code.borrow_mut().dup(0);
        }

        loader.push(products.first().unwrap());
        for (idx, (value, product)) in values.iter().zip(products.iter()).skip(1).enumerate() {
            loader.push(value);
            loader.code.borrow_mut().mulmod();
            if idx < values.len() - 2 {
                loader.code.borrow_mut().dup(0);
            }
            loader.code.borrow_mut().push(product.ptr()).mstore();
        }

        let inv = loader.invert(products.last().unwrap());

        loader.code.borrow_mut().push(loader.scalar_modulus);
        for _ in 2..values.len() {
            loader.code.borrow_mut().dup(0);
        }

        loader.push(&inv);
        for (value, product) in values.iter().rev().zip(
            products
                .iter()
                .rev()
                .skip(1)
                .map(Some)
                .chain(iter::once(None)),
        ) {
            if let Some(product) = product {
                loader.push(value);
                loader
                    .code
                    .borrow_mut()
                    .dup(2)
                    .dup(2)
                    .push(product.ptr())
                    .mload()
                    .mulmod()
                    .push(value.ptr())
                    .mstore()
                    .mulmod();
            } else {
                loader.code.borrow_mut().push(value.ptr()).mstore();
            }
        }
    }
}

impl<C> EcPointLoader<C> for Rc<EvmLoader>
where
    C: Curve + UncompressedEncoding<Uncompressed = [u8; 0x40]>,
    C::Scalar: PrimeField<Repr = [u8; 0x20]>,
{
    type LoadedEcPoint = EcPoint;

    fn ec_point_load_const(&self, value: &C) -> EcPoint {
        let bytes = value.to_uncompressed();
        let (x, y) = (
            U256::from_little_endian(&bytes[..32]),
            U256::from_little_endian(&bytes[32..]),
        );
        self.ec_point(Value::Constant((x, y)))
    }
}

impl<F: PrimeField<Repr = [u8; 0x20]>> ScalarLoader<F> for Rc<EvmLoader> {
    type LoadedScalar = Scalar;

    fn load_const(&self, value: &F) -> Scalar {
        self.scalar(Value::Constant(U256::from_little_endian(
            value.to_repr().as_slice(),
        )))
    }
}

impl<C> Loader<C> for Rc<EvmLoader>
where
    C: Curve + UncompressedEncoding<Uncompressed = [u8; 0x40]>,
    C::Scalar: PrimeField<Repr = [u8; 0x20]>,
{
    #[cfg(test)]
    fn start_cost_metering(&self, identifier: &str) {
        self.start_gas_metering(identifier)
    }

    #[cfg(test)]
    fn end_cost_metering(&self) {
        self.end_gas_metering()
    }
}
