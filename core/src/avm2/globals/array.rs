//! Array class

use crate::avm2::activation::Activation;
use crate::avm2::array::ArrayStorage;
use crate::avm2::class::Class;
use crate::avm2::method::{Method, NativeMethodImpl};
use crate::avm2::names::{Namespace, QName};
use crate::avm2::object::{array_allocator, ArrayObject, Object, TObject};
use crate::avm2::string::AvmString;
use crate::avm2::value::Value;
use crate::avm2::Error;
use bitflags::bitflags;
use gc_arena::{GcCell, MutationContext};
use std::cmp::{min, Ordering};
use std::mem::swap;

/// Implements `Array`'s instance initializer.
pub fn instance_init<'gc>(
    activation: &mut Activation<'_, 'gc, '_>,
    this: Option<Object<'gc>>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error> {
    if let Some(this) = this {
        activation.super_init(this, &[])?;

        if let Some(mut array) = this.as_array_storage_mut(activation.context.gc_context) {
            if args.len() == 1 {
                if let Some(expected_len) = args
                    .get(0)
                    .and_then(|v| v.as_number(activation.context.gc_context).ok())
                {
                    if expected_len < 0.0 || expected_len.is_nan() {
                        return Err("Length must be a positive integer".into());
                    }

                    array.set_length(expected_len as usize);

                    return Ok(Value::Undefined);
                }
            }

            for (i, arg) in args.iter().enumerate() {
                array.set(i, arg.clone());
            }
        }
    }

    Ok(Value::Undefined)
}

/// Implements `Array`'s class initializer.
pub fn class_init<'gc>(
    _activation: &mut Activation<'_, 'gc, '_>,
    _this: Option<Object<'gc>>,
    _args: &[Value<'gc>],
) -> Result<Value<'gc>, Error> {
    Ok(Value::Undefined)
}

/// Implements `Array.length`'s getter
pub fn length<'gc>(
    _activation: &mut Activation<'_, 'gc, '_>,
    this: Option<Object<'gc>>,
    _args: &[Value<'gc>],
) -> Result<Value<'gc>, Error> {
    if let Some(this) = this {
        if let Some(array) = this.as_array_storage() {
            return Ok(array.length().into());
        }
    }

    Ok(Value::Undefined)
}

/// Implements `Array.length`'s setter
pub fn set_length<'gc>(
    activation: &mut Activation<'_, 'gc, '_>,
    this: Option<Object<'gc>>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error> {
    if let Some(this) = this {
        if let Some(mut array) = this.as_array_storage_mut(activation.context.gc_context) {
            let size = args
                .get(0)
                .unwrap_or(&Value::Undefined)
                .coerce_to_u32(activation)?;
            array.set_length(size as usize);
        }
    }

    Ok(Value::Undefined)
}

/// Bundle an already-constructed `ArrayStorage` in an `Object`.
pub fn build_array<'gc>(
    activation: &mut Activation<'_, 'gc, '_>,
    array: ArrayStorage<'gc>,
) -> Result<Value<'gc>, Error> {
    Ok(ArrayObject::from_storage(activation, array)?.into())
}

/// Implements `Array.concat`
#[allow(clippy::map_clone)] //You can't clone `Option<Ref<T>>` without it
pub fn concat<'gc>(
    activation: &mut Activation<'_, 'gc, '_>,
    this: Option<Object<'gc>>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error> {
    let mut base_array = this
        .and_then(|this| this.as_array_storage().map(|a| a.clone()))
        .unwrap_or_else(|| ArrayStorage::new(0));

    for arg in args {
        if let Some(other_array) = arg.coerce_to_object(activation)?.as_array_storage() {
            base_array.append(&other_array);
        } else {
            base_array.push(arg.clone());
        }
    }

    build_array(activation, base_array)
}

/// Resolves array holes.
pub fn resolve_array_hole<'gc>(
    activation: &mut Activation<'_, 'gc, '_>,
    this: Object<'gc>,
    i: usize,
    item: Option<Value<'gc>>,
) -> Result<Value<'gc>, Error> {
    item.map(Ok).unwrap_or_else(|| {
        this.proto()
            .map(|p| {
                p.get_property(
                    p,
                    &QName::new(
                        Namespace::public(),
                        AvmString::new(activation.context.gc_context, i.to_string()),
                    ),
                    activation,
                )
            })
            .unwrap_or(Ok(Value::Undefined))
    })
}

pub fn join_inner<'gc, 'a, 'ctxt, C>(
    activation: &mut Activation<'a, 'gc, 'ctxt>,
    this: Option<Object<'gc>>,
    args: &[Value<'gc>],
    mut conv: C,
) -> Result<Value<'gc>, Error>
where
    C: for<'b> FnMut(Value<'gc>, &'b mut Activation<'a, 'gc, 'ctxt>) -> Result<Value<'gc>, Error>,
{
    let mut separator = args.get(0).cloned().unwrap_or(Value::Undefined);
    if separator == Value::Undefined {
        separator = ",".into();
    }

    if let Some(this) = this {
        if let Some(array) = this.as_array_storage() {
            let string_separator = separator.coerce_to_string(activation)?;
            let mut accum = Vec::with_capacity(array.length());

            for (i, item) in array.iter().enumerate() {
                let item = resolve_array_hole(activation, this, i, item)?;

                if matches!(item, Value::Undefined) || matches!(item, Value::Null) {
                    accum.push("".into());
                } else {
                    accum.push(
                        conv(item, activation)?
                            .coerce_to_string(activation)?
                            .to_string(),
                    );
                }
            }

            return Ok(AvmString::new(
                activation.context.gc_context,
                accum.join(&string_separator),
            )
            .into());
        }
    }

    Ok(Value::Undefined)
}

/// Implements `Array.join`
pub fn join<'gc>(
    activation: &mut Activation<'_, 'gc, '_>,
    this: Option<Object<'gc>>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error> {
    join_inner(activation, this, args, |v, _act| Ok(v))
}

/// Implements `Array.toString`
pub fn to_string<'gc>(
    activation: &mut Activation<'_, 'gc, '_>,
    this: Option<Object<'gc>>,
    _args: &[Value<'gc>],
) -> Result<Value<'gc>, Error> {
    join_inner(activation, this, &[",".into()], |v, _act| Ok(v))
}

/// Implements `Array.toLocaleString`
pub fn to_locale_string<'gc>(
    act: &mut Activation<'_, 'gc, '_>,
    this: Option<Object<'gc>>,
    _args: &[Value<'gc>],
) -> Result<Value<'gc>, Error> {
    join_inner(act, this, &[",".into()], |v, activation| {
        let o = v.coerce_to_object(activation)?;

        let tls = o.get_property(
            o,
            &QName::new(Namespace::public(), "toLocaleString"),
            activation,
        )?;

        tls.coerce_to_object(activation)?
            .call(Some(o), &[], activation, o.proto())
    })
}

/// Implements `Array.valueOf`
pub fn value_of<'gc>(
    activation: &mut Activation<'_, 'gc, '_>,
    this: Option<Object<'gc>>,
    _args: &[Value<'gc>],
) -> Result<Value<'gc>, Error> {
    join_inner(activation, this, &[",".into()], |v, _act| Ok(v))
}

/// An iterator that allows iterating over the contents of an array whilst also
/// executing user code.
///
/// Note that this does not actually implement `Iterator` as this struct needs
/// to share access to the activation with you. We can't claim your activation
/// and give it back in `next`, so we instead ask for it in `next`, which is
/// incompatible with the trait.
///
/// This technically works with Array-shaped, non-Array objects, since we
/// access arrays in this iterator the same way user code would. If it is
/// necessary to only work with Arrays, you must first check for array storage
/// before creating this iterator.
///
/// The primary purpose of `ArrayIter` is to maintain lock safety in the
/// presence of arbitrary user code. It is legal for, say, a method callback to
/// mutate the array under iteration. Normally, holding an `Iterator` on the
/// array while this happens would cause a panic; this code exists to prevent
/// that.
pub struct ArrayIter<'gc> {
    array_object: Object<'gc>,
    pub index: u32,
    pub rev_index: u32,
}

impl<'gc> ArrayIter<'gc> {
    /// Construct a new `ArrayIter`.
    pub fn new(
        activation: &mut Activation<'_, 'gc, '_>,
        array_object: Object<'gc>,
    ) -> Result<Self, Error> {
        Self::with_bounds(activation, array_object, 0, u32::MAX)
    }

    /// Construct a new `ArrayIter` that is bounded to a given range.
    pub fn with_bounds(
        activation: &mut Activation<'_, 'gc, '_>,
        array_object: Object<'gc>,
        start_index: u32,
        end_index: u32,
    ) -> Result<Self, Error> {
        let length = array_object
            .get_property(
                array_object,
                &QName::new(Namespace::public(), "length"),
                activation,
            )?
            .coerce_to_u32(activation)?;

        Ok(Self {
            array_object,
            index: start_index.min(length),
            rev_index: end_index.saturating_add(1).min(length),
        })
    }

    /// Get the next item from the front of the array
    ///
    /// Since this isn't a real iterator, this comes pre-enumerated; it yields
    /// a pair of the index and then the value.
    pub fn next(
        &mut self,
        activation: &mut Activation<'_, 'gc, '_>,
    ) -> Option<Result<(u32, Value<'gc>), Error>> {
        if self.index < self.rev_index {
            let i = self.index;

            self.index += 1;

            Some(
                self.array_object
                    .get_property(
                        self.array_object,
                        &QName::new(
                            Namespace::public(),
                            AvmString::new(activation.context.gc_context, i.to_string()),
                        ),
                        activation,
                    )
                    .map(|val| (i, val)),
            )
        } else {
            None
        }
    }

    /// Get the next item from the back of the array.
    ///
    /// Since this isn't a real iterator, this comes pre-enumerated; it yields
    /// a pair of the index and then the value.
    pub fn next_back(
        &mut self,
        activation: &mut Activation<'_, 'gc, '_>,
    ) -> Option<Result<(u32, Value<'gc>), Error>> {
        if self.index < self.rev_index {
            self.rev_index -= 1;

            let i = self.rev_index;

            Some(
                self.array_object
                    .get_property(
                        self.array_object,
                        &QName::new(
                            Namespace::public(),
                            AvmString::new(activation.context.gc_context, i.to_string()),
                        ),
                        activation,
                    )
                    .map(|val| (i, val)),
            )
        } else {
            None
        }
    }
}

/// Implements `Array.forEach`
pub fn for_each<'gc>(
    activation: &mut Activation<'_, 'gc, '_>,
    this: Option<Object<'gc>>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error> {
    if let Some(this) = this {
        let callback = args
            .get(0)
            .cloned()
            .unwrap_or(Value::Undefined)
            .coerce_to_object(activation)?;
        let receiver = args
            .get(1)
            .cloned()
            .unwrap_or(Value::Null)
            .coerce_to_object(activation)
            .ok();
        let mut iter = ArrayIter::new(activation, this)?;

        while let Some(r) = iter.next(activation) {
            let (i, item) = r?;

            callback.call(
                receiver,
                &[item, i.into(), this.into()],
                activation,
                receiver.and_then(|r| r.proto()),
            )?;
        }
    }

    Ok(Value::Undefined)
}

/// Implements `Array.map`
pub fn map<'gc>(
    activation: &mut Activation<'_, 'gc, '_>,
    this: Option<Object<'gc>>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error> {
    if let Some(this) = this {
        let callback = args
            .get(0)
            .cloned()
            .unwrap_or(Value::Undefined)
            .coerce_to_object(activation)?;
        let receiver = args
            .get(1)
            .cloned()
            .unwrap_or(Value::Null)
            .coerce_to_object(activation)
            .ok();
        let mut new_array = ArrayStorage::new(0);
        let mut iter = ArrayIter::new(activation, this)?;

        while let Some(r) = iter.next(activation) {
            let (i, item) = r?;
            let new_item = callback.call(
                receiver,
                &[item, i.into(), this.into()],
                activation,
                receiver.and_then(|r| r.proto()),
            )?;

            new_array.push(new_item);
        }

        return build_array(activation, new_array);
    }

    Ok(Value::Undefined)
}

/// Implements `Array.filter`
pub fn filter<'gc>(
    activation: &mut Activation<'_, 'gc, '_>,
    this: Option<Object<'gc>>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error> {
    if let Some(this) = this {
        let callback = args
            .get(0)
            .cloned()
            .unwrap_or(Value::Undefined)
            .coerce_to_object(activation)?;
        let receiver = args
            .get(1)
            .cloned()
            .unwrap_or(Value::Null)
            .coerce_to_object(activation)
            .ok();
        let mut new_array = ArrayStorage::new(0);
        let mut iter = ArrayIter::new(activation, this)?;

        while let Some(r) = iter.next(activation) {
            let (i, item) = r?;
            let is_allowed = callback
                .call(
                    receiver,
                    &[item.clone(), i.into(), this.into()],
                    activation,
                    receiver.and_then(|r| r.proto()),
                )?
                .coerce_to_boolean();

            if is_allowed {
                new_array.push(item);
            }
        }

        return build_array(activation, new_array);
    }

    Ok(Value::Undefined)
}

/// Implements `Array.every`
pub fn every<'gc>(
    activation: &mut Activation<'_, 'gc, '_>,
    this: Option<Object<'gc>>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error> {
    if let Some(this) = this {
        let callback = args
            .get(0)
            .cloned()
            .unwrap_or(Value::Undefined)
            .coerce_to_object(activation)?;
        let receiver = args
            .get(1)
            .cloned()
            .unwrap_or(Value::Null)
            .coerce_to_object(activation)
            .ok();
        let mut iter = ArrayIter::new(activation, this)?;

        while let Some(r) = iter.next(activation) {
            let (i, item) = r?;

            let result = callback
                .call(
                    receiver,
                    &[item, i.into(), this.into()],
                    activation,
                    receiver.and_then(|r| r.proto()),
                )?
                .coerce_to_boolean();

            if !result {
                return Ok(false.into());
            }
        }

        return Ok(true.into());
    }

    Ok(Value::Undefined)
}

/// Implements `Array.some`
pub fn some<'gc>(
    activation: &mut Activation<'_, 'gc, '_>,
    this: Option<Object<'gc>>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error> {
    if let Some(this) = this {
        let callback = args
            .get(0)
            .cloned()
            .unwrap_or(Value::Undefined)
            .coerce_to_object(activation)?;
        let receiver = args
            .get(1)
            .cloned()
            .unwrap_or(Value::Null)
            .coerce_to_object(activation)
            .ok();
        let mut iter = ArrayIter::new(activation, this)?;

        while let Some(r) = iter.next(activation) {
            let (i, item) = r?;

            let result = callback
                .call(
                    receiver,
                    &[item, i.into(), this.into()],
                    activation,
                    receiver.and_then(|r| r.proto()),
                )?
                .coerce_to_boolean();

            if result {
                return Ok(true.into());
            }
        }

        return Ok(false.into());
    }

    Ok(Value::Undefined)
}

/// Implements `Array.indexOf`
pub fn index_of<'gc>(
    activation: &mut Activation<'_, 'gc, '_>,
    this: Option<Object<'gc>>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error> {
    if let Some(this) = this {
        if let Some(array) = this.as_array_storage() {
            let search_val = args.get(0).cloned().unwrap_or(Value::Undefined);
            let from = args
                .get(1)
                .cloned()
                .unwrap_or_else(|| 0.into())
                .coerce_to_u32(activation)?;

            for (i, val) in array.iter().enumerate() {
                let val = resolve_array_hole(activation, this, i, val)?;
                if i >= from as usize && val == search_val {
                    return Ok(i.into());
                }
            }

            return Ok((-1).into());
        }
    }

    Ok(Value::Undefined)
}

/// Implements `Array.lastIndexOf`
pub fn last_index_of<'gc>(
    activation: &mut Activation<'_, 'gc, '_>,
    this: Option<Object<'gc>>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error> {
    if let Some(this) = this {
        if let Some(array) = this.as_array_storage() {
            let search_val = args.get(0).cloned().unwrap_or(Value::Undefined);
            let from = args
                .get(1)
                .cloned()
                .unwrap_or_else(|| i32::MAX.into())
                .coerce_to_u32(activation)?;

            for (i, val) in array.iter().enumerate().rev() {
                let val = resolve_array_hole(activation, this, i, val)?;
                if i <= from as usize && val == search_val {
                    return Ok(i.into());
                }
            }

            return Ok((-1).into());
        }
    }

    Ok(Value::Undefined)
}

/// Implements `Array.pop`
pub fn pop<'gc>(
    activation: &mut Activation<'_, 'gc, '_>,
    this: Option<Object<'gc>>,
    _args: &[Value<'gc>],
) -> Result<Value<'gc>, Error> {
    if let Some(this) = this {
        if let Some(mut array) = this.as_array_storage_mut(activation.context.gc_context) {
            return Ok(array.pop());
        }
    }

    Ok(Value::Undefined)
}

/// Implements `Array.push`
pub fn push<'gc>(
    activation: &mut Activation<'_, 'gc, '_>,
    this: Option<Object<'gc>>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error> {
    if let Some(this) = this {
        if let Some(mut array) = this.as_array_storage_mut(activation.context.gc_context) {
            for arg in args {
                array.push(arg.clone())
            }
        }
    }

    Ok(Value::Undefined)
}

pub fn reverse<'gc>(
    activation: &mut Activation<'_, 'gc, '_>,
    this: Option<Object<'gc>>,
    _args: &[Value<'gc>],
) -> Result<Value<'gc>, Error> {
    if let Some(this) = this {
        if let Some(mut array) = this.as_array_storage_mut(activation.context.gc_context) {
            let mut last_non_hole_index = None;
            for (i, val) in array.iter().enumerate() {
                if val.is_some() {
                    last_non_hole_index = Some(i + 1);
                }
            }

            let mut new_array = ArrayStorage::new(0);

            for i in
                (0..last_non_hole_index.unwrap_or_else(|| array.length().saturating_sub(1))).rev()
            {
                if let Some(value) = array.get(i) {
                    new_array.push(value)
                } else {
                    new_array.push_hole()
                }
            }

            new_array.set_length(array.length());

            swap(&mut *array, &mut new_array);

            return Ok(this.into());
        }
    }

    Ok(Value::Undefined)
}

/// Implements `Array.shift`
pub fn shift<'gc>(
    activation: &mut Activation<'_, 'gc, '_>,
    this: Option<Object<'gc>>,
    _args: &[Value<'gc>],
) -> Result<Value<'gc>, Error> {
    if let Some(this) = this {
        if let Some(mut array) = this.as_array_storage_mut(activation.context.gc_context) {
            return Ok(array.shift());
        }
    }

    Ok(Value::Undefined)
}

/// Implements `Array.unshift`
pub fn unshift<'gc>(
    activation: &mut Activation<'_, 'gc, '_>,
    this: Option<Object<'gc>>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error> {
    if let Some(this) = this {
        if let Some(mut array) = this.as_array_storage_mut(activation.context.gc_context) {
            for arg in args.iter().rev() {
                array.unshift(arg.clone())
            }
        }
    }

    Ok(Value::Undefined)
}

/// Resolve a possibly-negative array index to something guaranteed to be positive.
pub fn resolve_index<'gc>(
    activation: &mut Activation<'_, 'gc, '_>,
    index: Value<'gc>,
    length: usize,
) -> Result<usize, Error> {
    let index = index.coerce_to_i32(activation)?;

    Ok(if index < 0 {
        let offset = index as isize;
        length.saturating_sub((-offset) as usize)
    } else {
        (index as usize).min(length)
    })
}

/// Implements `Array.slice`
pub fn slice<'gc>(
    activation: &mut Activation<'_, 'gc, '_>,
    this: Option<Object<'gc>>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error> {
    if let Some(this) = this {
        let array_length = this.as_array_storage().map(|a| a.length());

        if let Some(array_length) = array_length {
            let actual_start = resolve_index(
                activation,
                args.get(0).cloned().unwrap_or_else(|| 0.into()),
                array_length,
            )?;
            let actual_end = resolve_index(
                activation,
                args.get(1).cloned().unwrap_or_else(|| 0xFFFFFF.into()),
                array_length,
            )?;
            let mut new_array = ArrayStorage::new(0);
            for i in actual_start..actual_end {
                if i >= array_length {
                    break;
                }

                new_array.push(resolve_array_hole(
                    activation,
                    this,
                    i,
                    this.as_array_storage().unwrap().get(i),
                )?);
            }

            return build_array(activation, new_array);
        }
    }

    Ok(Value::Undefined)
}

/// Implements `Array.splice`
pub fn splice<'gc>(
    activation: &mut Activation<'_, 'gc, '_>,
    this: Option<Object<'gc>>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error> {
    if let Some(this) = this {
        let array_length = this.as_array_storage().map(|a| a.length());

        if let Some(array_length) = array_length {
            if let Some(start) = args.get(0).cloned() {
                let actual_start = resolve_index(activation, start, array_length)?;
                let delete_count = args
                    .get(1)
                    .cloned()
                    .unwrap_or_else(|| array_length.into())
                    .coerce_to_i32(activation)?;

                let actual_end = min(array_length, actual_start + delete_count as usize);
                let args_slice = if args.len() > 2 {
                    args[2..].iter().cloned()
                } else {
                    [].iter().cloned()
                };

                let contents = this
                    .as_array_storage()
                    .map(|a| a.iter().collect::<Vec<Option<Value<'gc>>>>())
                    .unwrap();

                let mut resolved = Vec::with_capacity(contents.len());
                for (i, v) in contents.iter().enumerate() {
                    resolved.push(resolve_array_hole(activation, this, i, v.clone())?);
                }

                let removed = resolved
                    .splice(actual_start..actual_end, args_slice)
                    .collect::<Vec<Value<'gc>>>();
                let removed_array = ArrayStorage::from_args(&removed[..]);

                let mut resolved_array = ArrayStorage::from_args(&resolved[..]);

                if let Some(mut array) = this.as_array_storage_mut(activation.context.gc_context) {
                    swap(&mut *array, &mut resolved_array)
                }

                return build_array(activation, removed_array);
            }
        }
    }

    Ok(Value::Undefined)
}

bitflags! {
    /// The array options that a given sort operation may use.
    ///
    /// These are provided as a number by the VM and converted into bitflags.
    pub struct SortOptions: u8 {
        /// Request case-insensitive string value sort.
        const CASE_INSENSITIVE     = 1 << 0;

        /// Reverse the order of sorting.
        const DESCENDING           = 1 << 1;

        /// Reject sorting on arrays with multiple equivalent values.
        const UNIQUE_SORT          = 1 << 2;

        /// Yield a list of indices rather than sorting the array in-place.
        const RETURN_INDEXED_ARRAY = 1 << 3;

        /// Request numeric value sort.
        const NUMERIC              = 1 << 4;
    }
}

/// Identity closure shim which exists purely to decorate closure types with
/// the HRTB necessary to accept an activation.
fn constrain<'a, 'gc, 'ctxt, F>(f: F) -> F
where
    F: FnMut(&mut Activation<'a, 'gc, 'ctxt>, Value<'gc>, Value<'gc>) -> Result<Ordering, Error>,
{
    f
}

/// Sort array storage.
///
/// This function expects its values to have been pre-enumerated and
/// pre-resolved. They will be sorted in-place. It is the caller's
/// responsibility to place the resulting half of the sorted array wherever.
///
/// This function will reverse the sort order if `Descending` sort is requested.
///
/// This function will return `false` in the event that the `UniqueSort`
/// constraint has been violated (`sort_func` returned `Ordering::Equal`). In
/// this case, you should cancel the in-place sorting operation and return 0 to
/// the caller. In the event that this function yields a runtime error, the
/// contents of the `values` array will be sorted in a random order.
fn sort_inner<'a, 'gc, 'ctxt, C>(
    activation: &mut Activation<'a, 'gc, 'ctxt>,
    values: &mut [(usize, Value<'gc>)],
    options: SortOptions,
    mut sort_func: C,
) -> Result<bool, Error>
where
    C: FnMut(&mut Activation<'a, 'gc, 'ctxt>, Value<'gc>, Value<'gc>) -> Result<Ordering, Error>,
{
    let mut unique_sort_satisfied = true;
    let mut error_signal = Ok(());

    values.sort_unstable_by(|(_a_index, a), (_b_index, b)| {
        let unresolved_a = a.clone();
        let unresolved_b = b.clone();

        if matches!(unresolved_a, Value::Undefined) && matches!(unresolved_b, Value::Undefined) {
            unique_sort_satisfied = false;
            return Ordering::Equal;
        } else if matches!(unresolved_a, Value::Undefined) {
            return Ordering::Greater;
        } else if matches!(unresolved_b, Value::Undefined) {
            return Ordering::Less;
        }

        match sort_func(activation, a.clone(), b.clone()) {
            Ok(Ordering::Equal) => {
                unique_sort_satisfied = false;
                Ordering::Equal
            }
            Ok(v) if options.contains(SortOptions::DESCENDING) => v.reverse(),
            Ok(v) => v,
            Err(e) => {
                error_signal = Err(e);
                Ordering::Less
            }
        }
    });

    error_signal?;

    Ok(!options.contains(SortOptions::UNIQUE_SORT) || unique_sort_satisfied)
}

pub fn compare_string_case_sensitive<'gc>(
    activation: &mut Activation<'_, 'gc, '_>,
    a: Value<'gc>,
    b: Value<'gc>,
) -> Result<Ordering, Error> {
    let string_a = a.coerce_to_string(activation)?;
    let string_b = b.coerce_to_string(activation)?;

    Ok(string_a.cmp(&string_b))
}

pub fn compare_string_case_insensitive<'gc>(
    activation: &mut Activation<'_, 'gc, '_>,
    a: Value<'gc>,
    b: Value<'gc>,
) -> Result<Ordering, Error> {
    let string_a = a.coerce_to_string(activation)?.to_lowercase();
    let string_b = b.coerce_to_string(activation)?.to_lowercase();

    Ok(string_a.cmp(&string_b))
}

pub fn compare_numeric<'gc>(
    activation: &mut Activation<'_, 'gc, '_>,
    a: Value<'gc>,
    b: Value<'gc>,
) -> Result<Ordering, Error> {
    let num_a = a.coerce_to_number(activation)?;
    let num_b = b.coerce_to_number(activation)?;

    if num_a.is_nan() && num_b.is_nan() {
        Ok(Ordering::Equal)
    } else if num_a.is_nan() {
        Ok(Ordering::Greater)
    } else if num_b.is_nan() {
        Ok(Ordering::Less)
    } else {
        Ok(num_a.partial_cmp(&num_b).unwrap())
    }
}

/// Take a sorted set of values and produce the result requested by the caller.
fn sort_postprocess<'gc>(
    activation: &mut Activation<'_, 'gc, '_>,
    this: Object<'gc>,
    options: SortOptions,
    unique_satisfied: bool,
    values: Vec<(usize, Value<'gc>)>,
) -> Result<Value<'gc>, Error> {
    if unique_satisfied {
        if options.contains(SortOptions::RETURN_INDEXED_ARRAY) {
            return build_array(
                activation,
                ArrayStorage::from_storage(
                    values.iter().map(|(i, _v)| Some((*i).into())).collect(),
                ),
            );
        } else {
            if let Some(mut old_array) = this.as_array_storage_mut(activation.context.gc_context) {
                let new_vec = values
                    .iter()
                    .map(|(src, v)| {
                        if let Some(old_value) = old_array.get(*src) {
                            Some(old_value)
                        } else if !matches!(v, Value::Undefined) {
                            Some(v.clone())
                        } else {
                            None
                        }
                    })
                    .collect();

                let mut new_array = ArrayStorage::from_storage(new_vec);

                swap(&mut *old_array, &mut new_array);
            }

            return Ok(this.into());
        }
    }

    Ok(0.into())
}

/// Given a value, extract its array values.
///
/// If the value is not an array, this function yields `None`.
fn extract_array_values<'gc>(
    activation: &mut Activation<'_, 'gc, '_>,
    value: Value<'gc>,
) -> Result<Option<Vec<Value<'gc>>>, Error> {
    let object = value.coerce_to_object(activation).ok();
    let holey_vec = if let Some(object) = object {
        if let Some(field_array) = object.as_array_storage() {
            field_array.clone()
        } else {
            return Ok(None);
        }
    } else {
        return Ok(None);
    };

    let mut unholey_vec = Vec::with_capacity(holey_vec.length());
    for (i, v) in holey_vec.iter().enumerate() {
        unholey_vec.push(resolve_array_hole(
            activation,
            object.unwrap(),
            i,
            v.clone(),
        )?);
    }

    Ok(Some(unholey_vec))
}

/// Impl `Array.sort`
pub fn sort<'gc>(
    activation: &mut Activation<'_, 'gc, '_>,
    this: Option<Object<'gc>>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error> {
    if let Some(this) = this {
        let (compare_fnc, options) = if args.len() > 1 {
            (
                Some(
                    args.get(0)
                        .cloned()
                        .unwrap_or(Value::Undefined)
                        .coerce_to_object(activation)?,
                ),
                SortOptions::from_bits_truncate(
                    args.get(1)
                        .cloned()
                        .unwrap_or_else(|| 0.into())
                        .coerce_to_u32(activation)? as u8,
                ),
            )
        } else {
            (
                None,
                SortOptions::from_bits_truncate(
                    args.get(0)
                        .cloned()
                        .unwrap_or_else(|| 0.into())
                        .coerce_to_u32(activation)? as u8,
                ),
            )
        };

        let mut values = if let Some(values) = extract_array_values(activation, this.into())? {
            values
                .iter()
                .enumerate()
                .map(|(i, v)| (i, v.clone()))
                .collect::<Vec<(usize, Value<'gc>)>>()
        } else {
            return Ok(0.into());
        };

        let unique_satisfied = if let Some(v) = compare_fnc {
            sort_inner(
                activation,
                &mut values,
                options,
                constrain(|activation, a, b| {
                    let order = v
                        .call(None, &[a, b], activation, None)?
                        .coerce_to_number(activation)?;

                    if order > 0.0 {
                        Ok(Ordering::Greater)
                    } else if order < 0.0 {
                        Ok(Ordering::Less)
                    } else {
                        Ok(Ordering::Equal)
                    }
                }),
            )?
        } else if options.contains(SortOptions::NUMERIC) {
            sort_inner(activation, &mut values, options, compare_numeric)?
        } else if options.contains(SortOptions::CASE_INSENSITIVE) {
            sort_inner(
                activation,
                &mut values,
                options,
                compare_string_case_insensitive,
            )?
        } else {
            sort_inner(
                activation,
                &mut values,
                options,
                compare_string_case_sensitive,
            )?
        };

        return sort_postprocess(activation, this, options, unique_satisfied, values);
    }

    Ok(0.into())
}

/// Given a value, extract its array values.
///
/// If the value is not an array, it will be returned as if it was present in a
/// one-element array containing itself. This is intended for use with parsing
/// parameters which are optionally arrays.
fn extract_maybe_array_values<'gc>(
    activation: &mut Activation<'_, 'gc, '_>,
    value: Value<'gc>,
) -> Result<Vec<Value<'gc>>, Error> {
    Ok(extract_array_values(activation, value.clone())?.unwrap_or_else(|| vec![value]))
}

/// Given a value, extract its array values and coerce them to strings.
///
/// If the value is not an array, it will be returned as if it was present in a
/// one-element array containing itself. This is intended for use with parsing
/// parameters which are optionally arrays. The returned value will still be
/// coerced into a string in this case.
fn extract_maybe_array_strings<'gc>(
    activation: &mut Activation<'_, 'gc, '_>,
    value: Value<'gc>,
) -> Result<Vec<AvmString<'gc>>, Error> {
    let values = extract_maybe_array_values(activation, value)?;

    let mut out = Vec::with_capacity(values.len());
    for value in values {
        out.push(value.coerce_to_string(activation)?);
    }
    Ok(out)
}

/// Given a value, extract its array values and coerce them to SortOptions.
///
/// If the value is not an array, it will be returned as if it was present in a
/// one-element array containing itself. This is intended for use with parsing
/// parameters which are optionally arrays. The returned value will still be
/// coerced into a string in this case.
fn extract_maybe_array_sort_options<'gc>(
    activation: &mut Activation<'_, 'gc, '_>,
    value: Value<'gc>,
) -> Result<Vec<SortOptions>, Error> {
    let values = extract_maybe_array_values(activation, value)?;

    let mut out = Vec::with_capacity(values.len());
    for value in values {
        out.push(SortOptions::from_bits_truncate(
            value.coerce_to_u32(activation)? as u8,
        ));
    }
    Ok(out)
}

/// Impl `Array.sortOn`
pub fn sort_on<'gc>(
    activation: &mut Activation<'_, 'gc, '_>,
    this: Option<Object<'gc>>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error> {
    if let Some(this) = this {
        if let Some(field_names_value) = args.get(0).cloned() {
            let field_names = extract_maybe_array_strings(activation, field_names_value)?;
            let mut options = extract_maybe_array_sort_options(
                activation,
                args.get(1).cloned().unwrap_or_else(|| 0.into()),
            )?;

            let first_option = options.get(0).cloned().unwrap_or_else(SortOptions::empty)
                & (SortOptions::UNIQUE_SORT | SortOptions::RETURN_INDEXED_ARRAY);
            let mut values = if let Some(values) = extract_array_values(activation, this.into())? {
                values
                    .iter()
                    .enumerate()
                    .map(|(i, v)| (i, v.clone()))
                    .collect::<Vec<(usize, Value<'gc>)>>()
            } else {
                return Ok(0.into());
            };

            if options.len() < field_names.len() {
                options.resize(
                    field_names.len(),
                    options.last().cloned().unwrap_or_else(SortOptions::empty),
                );
            }

            let unique_satisfied = sort_inner(
                activation,
                &mut values,
                first_option,
                constrain(|activation, a, b| {
                    for (field_name, options) in field_names.iter().zip(options.iter()) {
                        let a_object = a.coerce_to_object(activation)?;
                        let a_field = a_object.get_property(
                            a_object,
                            &QName::new(Namespace::public(), *field_name),
                            activation,
                        )?;

                        let b_object = b.coerce_to_object(activation)?;
                        let b_field = b_object.get_property(
                            b_object,
                            &QName::new(Namespace::public(), *field_name),
                            activation,
                        )?;

                        let ord = if options.contains(SortOptions::NUMERIC) {
                            compare_numeric(activation, a_field, b_field)?
                        } else if options.contains(SortOptions::CASE_INSENSITIVE) {
                            compare_string_case_insensitive(activation, a_field, b_field)?
                        } else {
                            compare_string_case_sensitive(activation, a_field, b_field)?
                        };

                        if matches!(ord, Ordering::Equal) {
                            continue;
                        }

                        if options.contains(SortOptions::DESCENDING) {
                            return Ok(ord.reverse());
                        } else {
                            return Ok(ord);
                        }
                    }

                    Ok(Ordering::Equal)
                }),
            )?;

            return sort_postprocess(activation, this, first_option, unique_satisfied, values);
        }
    }

    Ok(0.into())
}

/// Construct `Array`'s class.
pub fn create_class<'gc>(mc: MutationContext<'gc, '_>) -> GcCell<'gc, Class<'gc>> {
    let class = Class::new(
        QName::new(Namespace::public(), "Array"),
        Some(QName::new(Namespace::public(), "Object").into()),
        Method::from_builtin(instance_init, "<Array instance initializer>", mc),
        Method::from_builtin(class_init, "<Array class initializer>", mc),
        mc,
    );

    let mut write = class.write(mc);

    write.set_instance_allocator(array_allocator);

    const PUBLIC_INSTANCE_METHODS: &[(&str, NativeMethodImpl)] = &[
        ("toString", to_string),
        ("toLocaleString", to_locale_string),
        ("valueOf", value_of),
    ];
    write.define_public_builtin_instance_methods(mc, PUBLIC_INSTANCE_METHODS);

    const PUBLIC_INSTANCE_PROPERTIES: &[(
        &str,
        Option<NativeMethodImpl>,
        Option<NativeMethodImpl>,
    )] = &[("length", Some(length), Some(set_length))];
    write.define_public_builtin_instance_properties(mc, PUBLIC_INSTANCE_PROPERTIES);

    const AS3_INSTANCE_METHODS: &[(&str, NativeMethodImpl)] = &[
        ("concat", concat),
        ("join", join),
        ("forEach", for_each),
        ("map", map),
        ("filter", filter),
        ("every", every),
        ("some", some),
        ("indexOf", index_of),
        ("lastIndexOf", last_index_of),
        ("pop", pop),
        ("push", push),
        ("reverse", reverse),
        ("shift", shift),
        ("unshift", unshift),
        ("slice", slice),
        ("splice", splice),
        ("sort", sort),
        ("sortOn", sort_on),
    ];
    write.define_as3_builtin_instance_methods(mc, AS3_INSTANCE_METHODS);

    const CONSTANTS: &[(&str, u32)] = &[
        (
            "CASEINSENSITIVE",
            SortOptions::CASE_INSENSITIVE.bits() as u32,
        ),
        ("DESCENDING", SortOptions::DESCENDING.bits() as u32),
        ("NUMERIC", SortOptions::NUMERIC.bits() as u32),
        (
            "RETURNINDEXEDARRAY",
            SortOptions::RETURN_INDEXED_ARRAY.bits() as u32,
        ),
        ("UNIQUESORT", SortOptions::UNIQUE_SORT.bits() as u32),
    ];
    write.define_public_constant_uint_class_traits(CONSTANTS);

    class
}
