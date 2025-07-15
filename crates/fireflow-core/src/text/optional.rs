use serde::Serialize;
use std::convert::Infallible;
use std::fmt;
use std::marker::PhantomData;
use std::mem;

/// Denotes a wrapped value which might be present.
///
/// In practice, this is an `Option` but more obvious in what it indicates. It
/// also allows some nice methods to be built on top of [`Option`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaybeValue<T>(pub Option<T>);

/// Denotes a wrapped value which is never absent.
#[derive(Clone, Serialize)]
pub struct AlwaysValue<T>(pub T);

/// Denotes a wrapped value which is never present.
#[derive(Clone, Serialize, Default)]
pub struct NeverValue<T>(PhantomData<T>);

/// Marker type for [`MaybeValue`].
#[derive(Clone, Serialize)]
pub struct MaybeFamily;

/// Marker type for [`AlwaysValue`].
#[derive(Clone, Serialize)]
pub struct AlwaysFamily;

/// Marker type for [`NeverValue`].
#[derive(Clone, Serialize)]
pub struct NeverFamily;

/// Encodes a type which might have something in it.
///
/// Intended to be used as a "type family" pattern.
pub trait MightHave {
    /// Concrete wrapper type which might have something
    type Wrapper<T>: From<T>;

    /// If true, the wrapper will always have a value.
    ///
    /// Obviously, the implementation needs to ensure this is in sync with the
    /// meaning of Wrapper<T>.
    const INFALLABLE: bool;

    /// Consume a value and return it as a wrapped value
    fn wrap<T>(n: T) -> Self::Wrapper<T> {
        n.into()
    }

    /// Consume a wrapped value and possibly return its contents.
    ///
    /// If no contents exist, return the original input so the caller can
    /// take back ownership.
    fn unwrap<T>(x: Self::Wrapper<T>) -> Result<T, Self::Wrapper<T>>;

    /// Borrow a wrapped value and return a new wrapper with borrowed contents.
    fn as_ref<T>(x: &Self::Wrapper<T>) -> Self::Wrapper<&T>;

    /// Consume a wrapped value and possibly return its contents.
    fn to_opt<T>(x: Self::Wrapper<T>) -> Option<T> {
        Self::unwrap(x).ok()
    }

    /// Borrow a wrapped value and possibly return borrowed contents.
    fn as_opt<T>(x: &Self::Wrapper<T>) -> Option<&T> {
        Self::to_opt(Self::as_ref(x))
    }
}

impl MightHave for MaybeFamily {
    type Wrapper<T> = MaybeValue<T>;
    const INFALLABLE: bool = false;

    fn unwrap<T>(x: Self::Wrapper<T>) -> Result<T, Self::Wrapper<T>> {
        x.0.ok_or(None.into())
    }

    fn as_ref<T>(x: &Self::Wrapper<T>) -> Self::Wrapper<&T> {
        x.as_ref()
    }
}

impl MightHave for AlwaysFamily {
    type Wrapper<T> = AlwaysValue<T>;
    const INFALLABLE: bool = true;

    fn unwrap<T>(x: Self::Wrapper<T>) -> Result<T, Self::Wrapper<T>> {
        Ok(x.0)
    }

    fn as_ref<T>(x: &Self::Wrapper<T>) -> Self::Wrapper<&T> {
        AlwaysValue(&x.0)
    }
}

impl<T> From<T> for AlwaysValue<T> {
    fn from(value: T) -> Self {
        AlwaysValue(value)
    }
}

impl<T> From<T> for MaybeValue<T> {
    fn from(value: T) -> Self {
        Some(value).into()
    }
}

impl<T> TryFrom<MaybeValue<T>> for AlwaysValue<T> {
    type Error = MaybeToAlwaysError;
    fn try_from(value: MaybeValue<T>) -> Result<Self, Self::Error> {
        value.0.ok_or(MaybeToAlwaysError).map(AlwaysValue)
    }
}

// This will never really fail but is implemented for symmetry with its inverse
impl<T> TryFrom<AlwaysValue<T>> for MaybeValue<T> {
    type Error = Infallible;
    fn try_from(value: AlwaysValue<T>) -> Result<Self, Infallible> {
        Ok(Some(value.0).into())
    }
}

impl<T> From<Option<T>> for MaybeValue<T> {
    fn from(value: Option<T>) -> Self {
        MaybeValue(value)
    }
}

impl<T> From<MaybeValue<T>> for Option<T> {
    fn from(value: MaybeValue<T>) -> Self {
        value.0
    }
}

impl<T: Copy> Copy for MaybeValue<T> {}

// slightly hacky thing to let us copy the inner bit while re-wrapping as option
// impl<T: Copy> From<OptionalKw<T>> for Option<T> {
//     fn from(value: &OptionalKw<T>) -> Self {
//         value.0
//     }
// }

impl<T> Default for MaybeValue<T> {
    fn default() -> MaybeValue<T> {
        MaybeValue(None)
    }
}

impl<V> MaybeValue<V> {
    pub fn as_ref(&self) -> MaybeValue<&V> {
        MaybeValue(self.0.as_ref())
    }

    pub fn as_ref_opt(&self) -> Option<&V> {
        self.0.as_ref()
    }

    pub fn map<F, W>(self, f: F) -> MaybeValue<W>
    where
        F: Fn(V) -> W,
    {
        MaybeValue(self.0.map(f))
    }

    /// Mutate thing in Option if present, and possibly unset Option entirely
    pub fn mut_or_unset<E, F, X>(&mut self, f: F) -> Result<Option<X>, E>
    where
        F: Fn(&mut V) -> Result<X, ClearOptionalOr<E>>,
    {
        match mem::replace(self, None.into()).0 {
            None => Ok(None),
            Some(mut x) => match f(&mut x) {
                Ok(y) => Ok(Some(y)),
                Err(ClearOptionalOr::Clear) => {
                    *self = None.into();
                    Ok(None)
                }
                Err(ClearOptionalOr::Error(e)) => Err(e),
            },
        }
    }

    pub fn mut_or_unset_nofail<F, X>(&mut self, f: F) -> Option<X>
    where
        F: Fn(&mut V) -> Result<X, ClearOptional>,
    {
        let Ok(x) = self.mut_or_unset(f);
        x
    }
}

impl<V, E> MaybeValue<Result<V, E>> {
    pub fn transpose(self) -> Result<MaybeValue<V>, E> {
        self.0.transpose().map(|x| x.into())
    }
}

pub type ClearOptional = ClearOptionalOr<Infallible>;

#[derive(Default)]
pub enum ClearOptionalOr<E> {
    #[default]
    Clear,
    Error(E),
}

impl<V: fmt::Display> MaybeValue<V> {
    pub fn as_opt_string(&self) -> Option<String> {
        self.0.as_ref().map(|x| x.to_string())
    }
}

impl<T: Serialize> Serialize for MaybeValue<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self.0.as_ref() {
            Some(x) => serializer.serialize_some(x),
            None => serializer.serialize_none(),
        }
    }
}

pub struct MaybeToAlwaysError;

impl fmt::Display for MaybeToAlwaysError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "optional keyword value is blank",)
    }
}
