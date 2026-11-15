//! A tensor: a declared shape and number type over a flat array.

use std::borrow::Cow;
use std::fmt;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::{Error, Result};

/// The number type a tensor holds, as its `dtype` names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Dtype {
    U8,
    I16,
    U32,
    F32,
    F64,
}

impl Dtype {
    /// The name a file gives it: `u8`, `i16`, `u32`, `f32` or `f64`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Dtype::U8 => "u8",
            Dtype::I16 => "i16",
            Dtype::U32 => "u32",
            Dtype::F32 => "f32",
            Dtype::F64 => "f64",
        }
    }
}

impl fmt::Display for Dtype {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

mod sealed {
    pub trait Sealed {}
}

/// A number type a [`Tensor`] can hold. Sealed: the five a `dtype` names.
pub trait Element:
    sealed::Sealed + Copy + PartialEq + fmt::Debug + Serialize + DeserializeOwned + JsonSchema
{
    /// The `dtype` that names this type.
    const DTYPE: Dtype;
    /// Whether a value can be written: JSON has no NaN or infinity.
    fn is_finite(self) -> bool {
        true
    }
}

macro_rules! element {
    ($ty:ty, $dtype:ident) => {
        impl sealed::Sealed for $ty {}
        impl Element for $ty {
            const DTYPE: Dtype = Dtype::$dtype;
        }
    };
    ($ty:ty, $dtype:ident, float) => {
        impl sealed::Sealed for $ty {}
        impl Element for $ty {
            const DTYPE: Dtype = Dtype::$dtype;
            fn is_finite(self) -> bool {
                <$ty>::is_finite(self)
            }
        }
    };
}

element!(u8, U8);
element!(i16, I16);
element!(u32, U32);
element!(f32, F32, float);
element!(f64, F64, float);

/// A declared shape and number type over a flat array of numbers. Written
/// `{"shape": [3, 44], "dtype": "u8", "data": [...]}`.
///
/// **Layout.** The data is row-major (C order) and contiguous, with no strides and no offset:
/// the last index runs fastest, so the element at index `[i0, i1, …, in]` of a tensor of shape
/// `[d0, d1, …, dn]` is `data[((i0 * d1 + i1) * d2 + i2) … * dn + in]`; for a matrix, element
/// `[r, c]` is `data[r * columns + c]`. That is ndarray's standard layout, so a later
/// conversion can take the `Vec` as it is.
///
/// **Invariants**, checked on construction ([`Tensor::from_parts`]) and on read, and refused
/// naming both sides:
/// - the product of the shape equals the length of the data;
/// - the rank is 2 or more. An empty shape (a scalar) and a rank-1 shape (a column, which is a
///   plain array) are refused. A dimension of size 0 is allowed: the tensor then holds no
///   numbers, as a grid with no lanes or no steps does;
/// - on read, the `dtype` names `T`, so a reader in any language knows what the numbers are
///   without the schema;
/// - a float is finite, as JSON cannot hold NaN or infinity.
///
/// The fields are private, so nothing can break these after construction: [`Tensor::data_mut`]
/// changes numbers in place, never their count.
#[derive(Debug, Clone, PartialEq)]
pub struct Tensor<T> {
    shape: Vec<usize>,
    data: Vec<T>,
}

/// How many numbers a shape holds, or `None` when that overflows.
fn volume(shape: &[usize]) -> Option<usize> {
    shape.iter().try_fold(1usize, |n, d| n.checked_mul(*d))
}

fn shown(shape: &[usize]) -> String {
    let dims: Vec<String> = shape.iter().map(usize::to_string).collect();
    format!("[{}]", dims.join(", "))
}

impl<T: Element> Tensor<T> {
    /// A tensor of `shape` over `data`, row-major ([layout](Tensor#layout)). The `Vec` is kept
    /// as it is, so [`Tensor::into_parts`] gives the same one back.
    ///
    /// # Errors
    ///
    /// When the shape has fewer than two dimensions, `data` does not hold exactly the product
    /// of the shape, or a float in it is not finite.
    pub fn from_parts(shape: Vec<usize>, data: Vec<T>) -> Result<Self> {
        if shape.len() < 2 {
            return Err(Error::Tensor(format!(
                "shape {} has {} dimension(s); a tensor has two or more, and a column is a \
                 plain array",
                shown(&shape),
                shape.len()
            )));
        }
        let want = volume(&shape).ok_or_else(|| {
            Error::Tensor(format!("shape {} is too large to hold", shown(&shape)))
        })?;
        if data.len() != want {
            return Err(Error::Tensor(format!(
                "shape {} holds {want} numbers, but data has {}",
                shown(&shape),
                data.len()
            )));
        }
        check_finite(&data)?;
        Ok(Tensor { shape, data })
    }

    /// A matrix of no rows and no columns: shape `[0, 0]`.
    #[must_use]
    pub fn empty() -> Self {
        Tensor {
            shape: vec![0, 0],
            data: Vec::new(),
        }
    }

    /// A tensor of `shape` with every number `value`.
    ///
    /// # Errors
    ///
    /// When the shape has fewer than two dimensions or is too large.
    pub fn filled(shape: Vec<usize>, value: T) -> Result<Self> {
        match volume(&shape) {
            Some(len) if shape.len() >= 2 => Self::from_parts(shape, vec![value; len]),
            _ => Self::from_parts(shape, Vec::new()),
        }
    }

    /// The size of each dimension, outermost first.
    #[must_use]
    pub fn shape(&self) -> &[usize] {
        &self.shape
    }

    /// The `dtype` that names `T`.
    #[must_use]
    pub const fn dtype(&self) -> Dtype {
        T::DTYPE
    }

    /// The numbers, row-major.
    #[must_use]
    pub fn data(&self) -> &[T] {
        &self.data
    }

    /// The numbers, row-major, to change in place. Their count is fixed by the shape.
    pub fn data_mut(&mut self) -> &mut [T] {
        &mut self.data
    }

    /// The shape and the numbers, row-major: the same `Vec` [`Tensor::from_parts`] was given.
    #[must_use]
    pub fn into_parts(self) -> (Vec<usize>, Vec<T>) {
        (self.shape, self.data)
    }
}

fn check_finite<T: Element>(data: &[T]) -> Result<()> {
    if let Some(i) = data.iter().position(|v| !v.is_finite()) {
        return Err(Error::Tensor(format!(
            "data[{i}] is {:?}, and a tensor's numbers are finite",
            data[i]
        )));
    }
    Ok(())
}

/// The fields as a file holds them, in the order they are written.
#[derive(Serialize)]
struct Written<'a, T> {
    shape: &'a [usize],
    dtype: Dtype,
    data: &'a [T],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Read<T> {
    shape: Vec<usize>,
    dtype: Dtype,
    data: Vec<T>,
}

impl<T: Element> Serialize for Tensor<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        check_finite(&self.data).map_err(serde::ser::Error::custom)?;
        Written {
            shape: &self.shape,
            dtype: T::DTYPE,
            data: &self.data,
        }
        .serialize(serializer)
    }
}

impl<'de, T: Element> Deserialize<'de> for Tensor<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let read = Read::<T>::deserialize(deserializer)?;
        if read.dtype != T::DTYPE {
            return Err(serde::de::Error::custom(format!(
                "invalid tensor: its dtype is {}, but this tensor holds {}",
                read.dtype,
                T::DTYPE
            )));
        }
        Tensor::from_parts(read.shape, read.data).map_err(serde::de::Error::custom)
    }
}

impl<T: Element> JsonSchema for Tensor<T> {
    fn schema_name() -> Cow<'static, str> {
        Cow::Owned(format!("Tensor_{}", T::DTYPE))
    }

    fn schema_id() -> Cow<'static, str> {
        Cow::Owned(format!("duet_core::Tensor<{}>", T::DTYPE))
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let item = generator.subschema_for::<T>();
        json_schema!({
            "type": "object",
            "description": "A declared shape and number type over a flat array, row-major (last index fastest). data holds exactly the product of shape.",
            "properties": {
                "shape": {
                    "type": "array",
                    "items": { "type": "integer", "minimum": 0 },
                    "minItems": 2,
                },
                "dtype": { "const": T::DTYPE.name() },
                "data": { "type": "array", "items": item },
            },
            "required": ["shape", "dtype", "data"],
            "additionalProperties": false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shape_that_does_not_match_its_data_is_refused_naming_both() {
        let err = Tensor::from_parts(vec![3, 4], vec![0u8; 11])
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("shape [3, 4] holds 12 numbers, but data has 11"),
            "{err}"
        );
        let err = Tensor::from_parts(vec![2, 2, 2], vec![0u8; 9])
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("shape [2, 2, 2] holds 8 numbers, but data has 9"),
            "{err}"
        );
        let t = Tensor::from_parts(vec![2, 2], vec![1i16, -2, 3, -4]).unwrap();
        assert_eq!(
            (t.shape(), t.dtype(), t.data()),
            (&[2, 2][..], Dtype::I16, &[1, -2, 3, -4][..])
        );
    }

    #[test]
    fn an_empty_or_rank_1_shape_is_refused_and_a_zero_sized_dimension_holds_nothing() {
        let err = Tensor::<u8>::from_parts(vec![], vec![7])
            .unwrap_err()
            .to_string();
        assert!(err.contains("shape [] has 0 dimension(s)"), "{err}");
        let err = Tensor::from_parts(vec![12], vec![0u8; 12])
            .unwrap_err()
            .to_string();
        assert!(err.contains("a column is a plain array"), "{err}");
        for shape in [vec![3, 0], vec![0, 5], vec![0, 0], vec![2, 0, 4]] {
            let t = Tensor::<u8>::from_parts(shape.clone(), Vec::new()).unwrap();
            assert_eq!((t.shape(), t.data().len()), (&shape[..], 0));
        }
        let err = Tensor::from_parts(vec![3, 0], vec![1u8])
            .unwrap_err()
            .to_string();
        assert!(err.contains("holds 0 numbers, but data has 1"), "{err}");
        assert_eq!(Tensor::<u8>::empty().shape(), &[0, 0]);
    }

    #[test]
    fn from_parts_then_into_parts_gives_back_the_same_vec() {
        let data: Vec<f32> = (0..12).map(|i| i as f32 * 0.5).collect();
        let pointer = data.as_ptr();
        let (shape, back) = Tensor::from_parts(vec![3, 4], data).unwrap().into_parts();
        assert_eq!(shape, [3, 4]);
        assert_eq!(back.as_ptr(), pointer, "the same allocation, not a copy");
        assert_eq!(back.len(), 12);
    }

    #[test]
    fn the_row_major_index_is_the_one_the_docs_give() {
        // [d0, d1, d2] = [2, 3, 4]: element [i0, i1, i2] is data[(i0 * 3 + i1) * 4 + i2].
        let (d1, d2) = (3, 4);
        let data: Vec<u32> = (0..24).collect();
        let t = Tensor::from_parts(vec![2, d1, d2], data).unwrap();
        for i0 in 0..2 {
            for i1 in 0..d1 {
                for i2 in 0..d2 {
                    let at = (i0 * d1 + i1) * d2 + i2;
                    // Each number was written as its own row-major position, last index fastest.
                    assert_eq!(t.data()[at], at as u32, "[{i0}, {i1}, {i2}]");
                }
            }
        }
        assert_eq!(t.data()[1], 1, "the last index runs fastest");
        assert_eq!(
            t.data()[d2],
            d2 as u32,
            "the next row starts after a whole row"
        );
    }

    #[test]
    fn a_float_that_is_not_finite_is_refused() {
        let err = Tensor::from_parts(vec![1, 2], vec![1.0f32, f32::NAN])
            .unwrap_err()
            .to_string();
        assert!(err.contains("data[1]") && err.contains("finite"), "{err}");
        let mut t = Tensor::from_parts(vec![1, 1], vec![1.0f64]).unwrap();
        t.data_mut()[0] = f64::INFINITY;
        assert!(
            check_finite(t.data()).is_err(),
            "and so is writing one set in place"
        );
    }
}
