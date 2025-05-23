// TODO maybe just use derive_more?

macro_rules! newtype_from {
    ($outer:ident, $inner:path) => {
        impl From<$inner> for $outer {
            fn from(value: $inner) -> Self {
                $outer(value)
            }
        }
    };
}

pub(crate) use newtype_from;

macro_rules! newtype_from_outer {
    ($outer:ident, $inner:path) => {
        impl From<$outer> for $inner {
            fn from(value: $outer) -> Self {
                value.0
            }
        }
    };
}

pub(crate) use newtype_from_outer;

macro_rules! newtype_disp {
    ($outer:ident) => {
        impl fmt::Display for $outer {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
                write!(f, "{}", self.0)
            }
        }
    };
}

pub(crate) use newtype_disp;

macro_rules! newtype_fromstr {
    ($outer:ident, $err:path) => {
        impl FromStr for $outer {
            type Err = $err;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                s.parse().map($outer)
            }
        }
    };
}

pub(crate) use newtype_fromstr;

macro_rules! newtype_asref {
    ($from:ident, $to:ident) => {
        impl AsRef<$to> for $from {
            fn as_ref(&self) -> &$to {
                self.0.as_ref()
            }
        }
    };
}

pub(crate) use newtype_asref;

macro_rules! match_many_to_one {
    ($value:expr, $root:ident, [$($variant:ident),*], $inner:ident, $action:block) => {
        match $value {
            $(
                $root::$variant($inner) => {
                    $action
                },
            )*
        }
    };
}

pub(crate) use match_many_to_one;

// macro_rules! enum_from {
//     ($inner:path, $enum_type:ident, $var:ident) => {
//         impl From<$inner> for $enum_type {
//             fn from(value: $inner) -> Self {
//                 $enum_type::$var(value)
//             }
//         }
//     };
// }

// pub(crate) use enum_from;
