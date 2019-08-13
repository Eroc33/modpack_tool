use serde::{Deserialize,Serialize};
use std::str::FromStr;

#[derive(Debug, PartialEq, Eq, Clone, Copy, Serialize, Deserialize)]
pub enum ReleaseStatus {
    Release,
    Beta,
    Alpha,
}

#[derive(Debug)]
pub struct UnknownVariant(String);

impl ReleaseStatus {
    pub fn value(self) -> &'static str {
        match self {
            Self::Release => "Release",
            Self::Beta => "Beta",
            Self::Alpha => "Alpha",
        }
    }

    pub fn accepts(self, other: Self) -> bool {
        other == self || match self {
            Self::Release => false,
            Self::Beta => Self::Release.accepts(other),
            Self::Alpha => Self::Beta.accepts(other),
        }
    }

    pub fn parse_short(s: &str) -> Result<Self, UnknownVariant>{
        match s {
            "R" => Ok(Self::Release),
            "B" => Ok(Self::Beta),
            "A" => Ok(Self::Alpha),
            s => Err(UnknownVariant(s.to_string())),
        }
    }
}

impl FromStr for ReleaseStatus {
    type Err = UnknownVariant;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Release" => Ok(Self::Release),
            "Beta" => Ok(Self::Beta),
            "Alpha" => Ok(Self::Alpha),
            s => Err(UnknownVariant(s.to_string())),
        }
    }
}