#[derive(Debug, Copy, Clone)]
pub enum Amount {
    Val(u64),
    Infinite,
}

impl PartialEq<u64> for Amount {
    fn eq(&self, other: &u64) -> bool {
        match self {
            Amount::Val(v) => v == other,
            Amount::Infinite => false,
        }
    }
}

impl PartialOrd<u64> for Amount {
    fn partial_cmp(&self, other: &u64) -> Option<std::cmp::Ordering> {
        match self {
            Amount::Val(v) => v.partial_cmp(other),
            Amount::Infinite => Some(std::cmp::Ordering::Greater),
        }
    }
}

impl Amount {
    pub fn is_zero(&self) -> bool {
        matches!(self, Amount::Val(0))
    }
}

impl Amount {
    pub(crate) fn decrement(self) -> Self {
        match self {
            Amount::Val(v) if v > 0 => Amount::Val(v - 1),
            _ => self,
        }
    }
}
