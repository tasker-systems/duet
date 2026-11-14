//! Exact lengths as fractions of a whole note.

use std::fmt;
use std::ops::{Add, Mul};

/// A non-negative fraction in lowest terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Ratio {
    num: u64,
    den: u64,
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

impl Ratio {
    pub(crate) const ZERO: Ratio = Ratio { num: 0, den: 1 };

    /// `num / den`. A zero denominator is the caller's bug; it is treated as 1.
    pub(crate) fn new(num: u64, den: u64) -> Ratio {
        let den = den.max(1);
        let g = gcd(num, den).max(1);
        Ratio {
            num: num / g,
            den: den / g,
        }
    }

    pub(crate) fn num(self) -> u64 {
        self.num
    }

    pub(crate) fn den(self) -> u64 {
        self.den
    }

    /// This many whole notes as ticks, when that is a whole number of ticks.
    pub(crate) fn ticks(self, ticks_per_whole: u64) -> Option<u64> {
        let scaled = u128::from(self.num) * u128::from(ticks_per_whole);
        let den = u128::from(self.den);
        (scaled % den == 0).then(|| u64::try_from(scaled / den).ok())?
    }

    pub(crate) fn as_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }
}

impl Mul for Ratio {
    type Output = Ratio;

    fn mul(self, rhs: Ratio) -> Ratio {
        // Cross-reduce first, so lengths stay small.
        let a = gcd(self.num, rhs.den).max(1);
        let b = gcd(rhs.num, self.den).max(1);
        Ratio::new(
            (self.num / a).saturating_mul(rhs.num / b),
            (self.den / b).saturating_mul(rhs.den / a),
        )
    }
}

impl Add for Ratio {
    type Output = Ratio;

    fn add(self, rhs: Ratio) -> Ratio {
        let g = gcd(self.den, rhs.den).max(1);
        let den = (self.den / g).saturating_mul(rhs.den);
        Ratio::new(
            (self.num.saturating_mul(rhs.den / g))
                .saturating_add(rhs.num.saturating_mul(self.den / g)),
            den,
        )
    }
}

impl fmt::Display for Ratio {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.den == 1 {
            write!(f, "{}", self.num)
        } else {
            write!(f, "{}/{}", self.num, self.den)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Ratio;

    #[test]
    fn arithmetic_is_exact() {
        assert_eq!(Ratio::new(2, 4), Ratio::new(1, 2));
        assert_eq!(Ratio::new(1, 8) * Ratio::new(3, 2), Ratio::new(3, 16));
        assert_eq!(Ratio::new(1, 4) + Ratio::new(3, 8), Ratio::new(5, 8));
        assert_eq!(Ratio::new(1, 8).ticks(1920), Some(240));
        assert_eq!(Ratio::new(1, 7).ticks(1920), None);
        assert_eq!(Ratio::new(1, 12).ticks(1920), Some(160));
    }
}
