mod alloc;
mod block;
mod bump;
mod constants;
mod internal;
mod pointers;

pub use alloc::*;
use block::*;
pub use bump::StickyImmixHeap;
pub use pointers::*;

use std::{
    cell::{Cell, UnsafeCell},
    collections::HashMap,
    hash::{BuildHasher, Hasher},
    marker::PhantomData,
    mem::replace,
    ops::{BitXor, Deref},
    ptr::{NonNull, write},
    slice::from_raw_parts_mut,
};

use thiserror::Error;

use crate::{
    interpreter::RuntimeError,
    values::{
        Literal,
        value::{Array, Function, Text},
    },
};

#[derive(Clone, Copy)]
pub enum Mark {
    Allocated,
}

#[derive(Clone, Copy)]
pub enum SizeClass {
    Small,
    Medium,
    Large,
}

impl SizeClass {
    pub fn get_for_size(size: usize) -> Self {
        match (size > constants::LINE_SIZE, size > constants::BLOCK_SIZE) {
            (false, _) => Self::Small,
            (_, false) => Self::Medium,
            _ => Self::Large,
        }
    }
}

#[derive(Error, Debug, Clone, Copy)]
pub enum AllocError {
    #[error("Bad request")]
    BadRequest,
    #[error("Out of memory")]
    OOM,
}

pub trait AllocTypeId: Copy + Clone {}

#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TypeList {
    Number,
    String,
    Boolean,
    Function,
    Array,
}

impl AllocTypeId for TypeList {}

#[allow(dead_code)]
struct TypeHeader {
    size: u32,
    mark: Mark,
    type_id: TypeList,
    size_class: SizeClass,
}

impl AllocHeader for TypeHeader {
    type TypeId = TypeList;

    fn new<O: AllocObject<Self::TypeId>>(size: u32, size_class: SizeClass, mark: Mark) -> Self {
        Self {
            size,
            mark,
            type_id: O::TYPE_ID,
            size_class,
        }
    }

    fn new_array(size: super::tape::ArraySize, size_class: SizeClass, mark: Mark) -> Self {
        Self {
            size,
            mark,
            type_id: TypeList::Array,
            size_class,
        }
    }

    fn type_id(&self) -> Self::TypeId {
        self.type_id
    }
}

/// This enum defines an implementor of the [`AllocTypeId`] trait to be used when allocating memory
/// in the heap for Literals.
#[derive(Copy, Clone)]
pub enum LiteralList {
    Number,
    String,
    Invalid,
}

/// A simple header implementor of [`AllocHeader`], used for Read Only data for storing literals in
/// the heap.
#[allow(dead_code)]
pub struct LiteralHeader {
    size: u32,
    mark: Mark,
    type_id: LiteralList,
    size_class: SizeClass,
}

pub struct ROData<'mem> {
    heap: &'mem StickyImmixHeap<LiteralHeader>,
}

impl Clone for ROData<'_> {
    fn clone(&self) -> Self {
        Self { heap: self.heap }
    }
}

impl AllocTypeId for LiteralList {}

impl AllocObject<LiteralList> for f64 {
    const TYPE_ID: LiteralList = LiteralList::Number;
}
impl AllocObject<TypeList> for f64 {
    const TYPE_ID: TypeList = TypeList::Number;
}
impl AllocObject<TypeList> for bool {
    const TYPE_ID: TypeList = TypeList::Boolean;
}

impl AllocObject<LiteralList> for String {
    const TYPE_ID: LiteralList = LiteralList::String;
}

impl AllocHeader for LiteralHeader {
    type TypeId = LiteralList;

    fn new<O: AllocObject<Self::TypeId>>(
        size: ArraySize,
        size_class: SizeClass,
        mark: Mark,
    ) -> Self {
        Self {
            size,
            size_class,
            mark,
            type_id: O::TYPE_ID,
        }
    }

    fn new_array(size: ArraySize, size_class: SizeClass, mark: Mark) -> Self {
        Self {
            size,
            size_class,
            mark,
            type_id: LiteralList::Invalid,
        }
    }

    fn type_id(&self) -> Self::TypeId {
        self.type_id
    }
}

impl MutatorScope for ROData<'_> {}

impl<'mem> ROData<'mem> {
    /// Parse this `token` into a literal or return `None`.
    ///
    /// This function allocates space in [`Self::heap`] for a [`TaggedCellPtr`] pointing to the
    /// parsed token if successful, or else nothing is allocated and [`None`] is returned.
    pub fn alloc(&'mem self, token: String) -> Option<TaggedCellPtr> {
        let token = if token.chars().nth(0) == Some('\"') {
            let rest = &token[1..];
            let j = rest.find('"')?;
            Some(rest[..j].to_string())
        } else if token.chars().nth(0).is_some_and(char::is_numeric) {
            Some(
                token
                    .chars()
                    .take_while(|d| d.is_numeric() || *d == '.')
                    .collect::<String>(),
            )
        } else {
            None
        }?;

        // Try parsing as a number first, otherwise treat as string
        let literal = if let Ok(n) = token.parse::<f64>() {
            Literal::Number(n)
        } else {
            Literal::String(token)
        };

        match literal {
            Literal::Number(n) => {
                let tagged = TaggedPtr::number(n);
                Some(TaggedCellPtr::new(tagged))
            }
            Literal::String(s) => {
                let raw = self.heap.alloc(Text::from(s)).ok()?;
                let tagged = TaggedPtr::object(raw.to_void());
                Some(TaggedCellPtr::new(tagged))
            }
        }
    }

    pub fn new_with(heap: &'mem StickyImmixHeap<LiteralHeader>) -> ROData<'mem> {
        Self { heap }
    }
}

pub struct FxHasher {
    hash: usize,
}
const K: usize = 0x517cc1b727220a95;

impl Hasher for FxHasher {
    fn write(&mut self, bytes: &[u8]) {
        let i = bytes
            .iter()
            .take(8)
            .fold(0u64, |acc, &b| (acc << 8) | b as u64) as usize;
        self.hash = self.hash.rotate_left(5).bitxor(i).wrapping_mul(K);
    }

    fn finish(&self) -> u64 {
        self.hash as u64
    }
}

pub struct BuildFxHasher {}
impl BuildHasher for BuildFxHasher {
    type Hasher = FxHasher;

    fn build_hasher(&self) -> Self::Hasher {
        FxHasher { hash: 0 }
    }
}

pub struct Tape {
    heap: StickyImmixHeap<TypeHeader>,
}

impl Tape {
    #[must_use]
    fn new() -> Self {
        let heap: StickyImmixHeap<TypeHeader> = StickyImmixHeap::new();
        Self { heap }
    }

    fn alloc<T>(&self, object: T) -> Result<RawPtr<T>, RuntimeError>
    where
        T: AllocObject<TypeList>,
    {
        Ok(self.heap.alloc(object)?)
    }

    fn alloc_tagged<T>(&self, object: T) -> Result<TaggedPtr, RuntimeError>
    where
        FatPtr: From<RawPtr<T>>,
        T: AllocObject<TypeList>,
    {
        Ok(TaggedPtr::from(FatPtr::from(self.alloc(object)?)))
    }

    pub fn get_value(
        &self,
        idx: ArraySize,
        guard: &dyn MutatorScope,
        map: &HashMap<ArraySize, TaggedCellPtr, BuildFxHasher>,
    ) -> Option<super::Value> {
        let ptr = map.get(&idx)?;
        let value = ptr.get(guard).get_value().into();
        Some(value)
    }

    pub fn upsert_value(
        &self,
        idx: ArraySize,
        value: super::Value,
        map: &mut HashMap<ArraySize, TaggedCellPtr, BuildFxHasher>,
    ) -> Result<(), RuntimeError> {
        let ptr: TaggedPtr = match value {
            super::Value::String(s) => {
                let text = Text::from(s);
                self.alloc_tagged(text)?
            }
            super::Value::Err(e) => {
                let text = Text::from(e.to_string());
                self.alloc_tagged(text)?
            }
            super::Value::Number(n) => self.alloc_tagged(n)?,
            super::Value::Boolean(b) => self.alloc_tagged(b)?,
        };

        if let Some(val) = map.get_mut(&idx) {
            *val = TaggedCellPtr::new(ptr);
        } else {
            map.insert(idx, TaggedCellPtr::new(ptr));
        }

        Ok(())
    }
}

impl From<AllocError> for RuntimeError {
    fn from(value: AllocError) -> Self {
        RuntimeError::AllocError(value.to_string())
    }
}

pub struct MutatorView<'memory> {
    tape: Tape,
    literals: ROData<'memory>,
}

impl MutatorScope for MutatorView<'_> {}

impl<'memory> MutatorView<'memory> {
    pub fn alloc<T>(&self, object: T) -> Result<ScopedPtr<'_, T>, RuntimeError>
    where
        T: AllocObject<TypeList>,
    {
        Ok(ScopedPtr::new(
            self,
            self.tape.alloc(object)?.scoped_ref(self),
        ))
    }

    pub fn alloc_tagged<T>(&self, object: T) -> Result<TaggedScopedPtr<'_>, RuntimeError>
    where
        T: AllocObject<TypeList>,
        FatPtr: From<RawPtr<T>>,
    {
        let raw = self.tape.alloc_tagged(object)?;
        Ok(TaggedScopedPtr::new(self, raw))
    }

    pub fn get_tape(&'memory self) -> &'memory Tape {
        &self.tape
    }

    pub fn get_data(&self) -> &'memory ROData<'_> {
        &self.literals
    }

    pub fn new_with(literals: &'memory StickyImmixHeap<LiteralHeader>) -> Self {
        let tape = Tape::new();
        let rodata = ROData::new_with(literals);

        Self {
            tape,
            literals: rodata,
        }
    }
}
