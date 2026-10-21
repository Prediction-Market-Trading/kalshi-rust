use crate::{Error, Result};
use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{fmt, str::FromStr};

macro_rules! fixed {
    ($name:ident, $scale:expr, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash)]
        pub struct $name(Decimal);

        impl $name {
            /// Construct from an exact decimal, rejecting excess scale.
            pub fn new(value: Decimal) -> Result<Self> {
                if value.scale() > $scale {
                    return Err(Error::FixedPoint("too many decimal places"));
                }
                Ok(Self(value))
            }
            /// Access the exact decimal for checked arithmetic.
            pub fn decimal(self) -> Decimal {
                self.0
            }
        }
        impl FromStr for $name {
            type Err = Error;
            fn from_str(value: &str) -> Result<Self> {
                let digits = value.strip_prefix('-').unwrap_or(value);
                let mut parts = digits.split('.');
                let whole = parts.next().unwrap_or("");
                let fraction = parts.next();
                if whole.is_empty()
                    || !whole.bytes().all(|b| b.is_ascii_digit())
                    || fraction
                        .is_some_and(|s| s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()))
                    || parts.next().is_some()
                {
                    return Err(Error::FixedPoint("expected a plain decimal string"));
                }
                Self::new(
                    Decimal::from_str_exact(value)
                        .map_err(|_| Error::FixedPoint("decimal is out of range"))?,
                )
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
        impl Serialize for $name {
            fn serialize<S: Serializer>(
                &self,
                serializer: S,
            ) -> std::result::Result<S::Ok, S::Error> {
                serializer.serialize_str(&self.to_string())
            }
        }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(
                deserializer: D,
            ) -> std::result::Result<Self, D::Error> {
                String::deserialize(deserializer)?
                    .parse()
                    .map_err(serde::de::Error::custom)
            }
        }
    };
}
fixed!(FixedPointDollars, 6, "Exact dollar string, with up to six response decimal places. Request quote prices generally accept two to four; market tick rules apply. No floating-point conversions are performed.");
fixed!(FixedPointCount, 2, "Exact contract count string, with up to two decimal places. Signed values support positions and deltas. Request positivity rules are enforced by the exchange.");
