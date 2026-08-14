use std::num::NonZeroU32;

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ValueKind {
    Null = 0,
    Bool = 1,
    Number = 2,
    Integer = 3,
    Date = 4,
    RequestHeapRef = 5,
    ActorStateRef = 6,
    ConstRef = 7,
    ResourceRef = 8,
    CallbackClosureRef = 9,
}

impl ValueKind {
    const fn from_discriminant(discriminant: u8) -> Option<Self> {
        match discriminant {
            0 => Some(Self::Null),
            1 => Some(Self::Bool),
            2 => Some(Self::Number),
            3 => Some(Self::Integer),
            4 => Some(Self::Date),
            5 => Some(Self::RequestHeapRef),
            6 => Some(Self::ActorStateRef),
            7 => Some(Self::ConstRef),
            8 => Some(Self::ResourceRef),
            9 => Some(Self::CallbackClosureRef),
            _ => None,
        }
    }

    const fn requires_type_tag(self) -> bool {
        matches!(
            self,
            Self::RequestHeapRef
                | Self::ActorStateRef
                | Self::ConstRef
                | Self::ResourceRef
                | Self::CallbackClosureRef
        )
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CompactTypeTag(NonZeroU32);

impl CompactTypeTag {
    /// Encodes one exact linked `TypeIndex` as `index + 1`.
    ///
    /// Raw zero is reserved for an absent tag on immediate carriers, so the
    /// largest `u32` cannot be represented and fails closed here.
    pub const fn try_from_type_index(type_index: u32) -> Option<Self> {
        let Some(encoded) = type_index.checked_add(1) else {
            return None;
        };
        let Some(encoded) = NonZeroU32::new(encoded) else {
            return None;
        };
        Some(Self(encoded))
    }

    /// Returns the exact logical linked type index carried by this present tag.
    pub const fn type_index(self) -> u32 {
        self.0.get() - 1
    }

    const fn from_encoded(encoded: u32) -> Option<Self> {
        match NonZeroU32::new(encoded) {
            Some(encoded) => Some(Self(encoded)),
            None => None,
        }
    }

    const fn encoded(self) -> u32 {
        self.0.get()
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ValueFlags(u8);

impl ValueFlags {
    pub const fn new(bits: u8) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u8 {
        self.0
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VmHandle(u64);

impl VmHandle {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

const FLAGS_SHIFT: u32 = 8;
const TYPE_TAG_SHIFT: u32 = 16;
const KIND_MASK: u64 = u8::MAX as u64;
const FLAGS_MASK: u64 = (u8::MAX as u64) << FLAGS_SHIFT;
const TYPE_TAG_MASK: u64 = (u32::MAX as u64) << TYPE_TAG_SHIFT;
const RESERVED_MASK: u64 = !(KIND_MASK | FLAGS_MASK | TYPE_TAG_MASK);

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ValueSlot {
    payload: u64,
    metadata: u64,
}

impl ValueSlot {
    pub const fn null() -> Self {
        Self::immediate(0, ValueKind::Null)
    }

    pub const fn bool(value: bool) -> Self {
        Self::immediate(value as u64, ValueKind::Bool)
    }

    pub const fn number(value: f64) -> Self {
        Self::immediate(value.to_bits(), ValueKind::Number)
    }

    pub const fn integer(value: i64) -> Self {
        Self::immediate(value as u64, ValueKind::Integer)
    }

    pub const fn date(value: i64) -> Self {
        Self::immediate(value as u64, ValueKind::Date)
    }

    pub const fn request_heap_ref(
        handle: VmHandle,
        compact_type_tag: CompactTypeTag,
        flags: ValueFlags,
    ) -> Self {
        Self::reference(handle, ValueKind::RequestHeapRef, compact_type_tag, flags)
    }

    pub const fn actor_state_ref(
        handle: VmHandle,
        compact_type_tag: CompactTypeTag,
        flags: ValueFlags,
    ) -> Self {
        Self::reference(handle, ValueKind::ActorStateRef, compact_type_tag, flags)
    }

    pub const fn const_ref(
        handle: VmHandle,
        compact_type_tag: CompactTypeTag,
        flags: ValueFlags,
    ) -> Self {
        Self::reference(handle, ValueKind::ConstRef, compact_type_tag, flags)
    }

    pub const fn resource_ref(
        handle: VmHandle,
        compact_type_tag: CompactTypeTag,
        flags: ValueFlags,
    ) -> Self {
        Self::reference(handle, ValueKind::ResourceRef, compact_type_tag, flags)
    }

    pub const fn callback_closure_ref(
        handle: VmHandle,
        compact_type_tag: CompactTypeTag,
        flags: ValueFlags,
    ) -> Self {
        Self::reference(
            handle,
            ValueKind::CallbackClosureRef,
            compact_type_tag,
            flags,
        )
    }

    pub const fn kind(self) -> Option<ValueKind> {
        if self.metadata & RESERVED_MASK != 0 {
            return None;
        }
        let Some(kind) = ValueKind::from_discriminant((self.metadata & KIND_MASK) as u8) else {
            return None;
        };
        if kind.requires_type_tag() != self.compact_type_tag().is_some() {
            return None;
        }
        Some(kind)
    }

    pub const fn flags(self) -> ValueFlags {
        ValueFlags::new(((self.metadata & FLAGS_MASK) >> FLAGS_SHIFT) as u8)
    }

    /// Decodes the optional tag field. [`Self::kind`] additionally validates
    /// that immediate kinds have no tag and reference kinds have one.
    pub const fn compact_type_tag(self) -> Option<CompactTypeTag> {
        CompactTypeTag::from_encoded(((self.metadata & TYPE_TAG_MASK) >> TYPE_TAG_SHIFT) as u32)
    }

    pub const fn is_null(&self) -> bool {
        matches!(self.as_null(), Some(()))
    }

    pub const fn as_null(&self) -> Option<()> {
        if matches!(self.kind(), Some(ValueKind::Null)) && self.payload == 0 {
            Some(())
        } else {
            None
        }
    }

    pub const fn as_bool(&self) -> Option<bool> {
        if !matches!(self.kind(), Some(ValueKind::Bool)) {
            return None;
        }
        match self.payload {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }
    }

    pub const fn as_number(&self) -> Option<f64> {
        if matches!(self.kind(), Some(ValueKind::Number)) {
            Some(f64::from_bits(self.payload))
        } else {
            None
        }
    }

    pub const fn as_integer(&self) -> Option<i64> {
        if matches!(self.kind(), Some(ValueKind::Integer)) {
            Some(self.payload as i64)
        } else {
            None
        }
    }

    pub const fn as_date(&self) -> Option<i64> {
        if matches!(self.kind(), Some(ValueKind::Date)) {
            Some(self.payload as i64)
        } else {
            None
        }
    }

    pub const fn as_handle(&self) -> Option<VmHandle> {
        match self.kind() {
            Some(
                ValueKind::RequestHeapRef
                | ValueKind::ActorStateRef
                | ValueKind::ConstRef
                | ValueKind::ResourceRef
                | ValueKind::CallbackClosureRef,
            ) => Some(VmHandle::new(self.payload)),
            _ => None,
        }
    }

    pub const fn as_request_heap_ref(&self) -> Option<VmHandle> {
        self.handle_for(ValueKind::RequestHeapRef)
    }

    pub const fn as_actor_state_ref(&self) -> Option<VmHandle> {
        self.handle_for(ValueKind::ActorStateRef)
    }

    pub const fn as_const_ref(&self) -> Option<VmHandle> {
        self.handle_for(ValueKind::ConstRef)
    }

    pub const fn as_resource_ref(&self) -> Option<VmHandle> {
        self.handle_for(ValueKind::ResourceRef)
    }

    pub const fn as_callback_closure_ref(&self) -> Option<VmHandle> {
        self.handle_for(ValueKind::CallbackClosureRef)
    }

    const fn immediate(payload: u64, kind: ValueKind) -> Self {
        Self::new(payload, kind, None, ValueFlags::new(0))
    }

    const fn reference(
        handle: VmHandle,
        kind: ValueKind,
        compact_type_tag: CompactTypeTag,
        flags: ValueFlags,
    ) -> Self {
        Self::new(handle.get(), kind, Some(compact_type_tag), flags)
    }

    const fn new(
        payload: u64,
        kind: ValueKind,
        compact_type_tag: Option<CompactTypeTag>,
        flags: ValueFlags,
    ) -> Self {
        let encoded_type_tag = match compact_type_tag {
            Some(tag) => tag.encoded(),
            None => 0,
        };
        let metadata = (kind as u64)
            | ((flags.bits() as u64) << FLAGS_SHIFT)
            | ((encoded_type_tag as u64) << TYPE_TAG_SHIFT);
        Self { payload, metadata }
    }

    const fn handle_for(&self, expected: ValueKind) -> Option<VmHandle> {
        match self.kind() {
            Some(actual) if actual as u8 == expected as u8 => Some(VmHandle::new(self.payload)),
            _ => None,
        }
    }

    #[cfg(test)]
    const fn from_raw_parts_for_test(payload: u64, metadata: u64) -> Self {
        Self { payload, metadata }
    }

    #[cfg(test)]
    const fn metadata_for_test(self) -> u64 {
        self.metadata
    }
}

#[cfg(test)]
mod tests {
    use std::mem::{align_of, size_of};

    use super::{
        CompactTypeTag, ValueFlags, ValueKind, ValueSlot, VmHandle, RESERVED_MASK, TYPE_TAG_MASK,
    };

    type ReferenceCase = (ValueKind, ValueSlot, fn(&ValueSlot) -> Option<VmHandle>);

    fn tag(type_index: u32) -> CompactTypeTag {
        CompactTypeTag::try_from_type_index(type_index).expect("type index must fit compact tag")
    }

    #[test]
    fn value_slot_layout_is_two_words() {
        assert_eq!(size_of::<ValueSlot>(), 16);
        assert_eq!(align_of::<ValueSlot>(), 8);
        assert_eq!(size_of::<CompactTypeTag>(), size_of::<u32>());
        assert_eq!(size_of::<Option<CompactTypeTag>>(), size_of::<u32>());
    }

    #[test]
    fn immediate_values_round_trip() {
        let nan_bits = 0x7ff8_0000_0000_0042;
        let number = ValueSlot::number(f64::from_bits(nan_bits));
        let immediates = [
            ValueSlot::null(),
            ValueSlot::bool(true),
            number,
            ValueSlot::integer(i64::MIN),
            ValueSlot::date(-1_234_567),
        ];

        assert_eq!(ValueSlot::null().as_null(), Some(()));
        assert!(ValueSlot::null().is_null());
        assert_eq!(ValueSlot::bool(false).as_bool(), Some(false));
        assert_eq!(ValueSlot::bool(true).as_bool(), Some(true));
        assert_eq!(number.as_number().map(f64::to_bits), Some(nan_bits));
        assert_eq!(ValueSlot::integer(i64::MIN).as_integer(), Some(i64::MIN));
        assert_eq!(ValueSlot::date(-1_234_567).as_date(), Some(-1_234_567));
        for slot in immediates {
            assert_eq!(slot.compact_type_tag(), None);
            assert_eq!(slot.flags(), ValueFlags::new(0));
        }
    }

    #[test]
    fn reference_values_round_trip_kind_handle_tag_and_flags() {
        let handle = VmHandle::new(0xfedc_ba98_7654_3210);
        let tag = tag(0x89ab_cdef);
        let flags = ValueFlags::new(0xa5);
        let cases: [ReferenceCase; 5] = [
            (
                ValueKind::RequestHeapRef,
                ValueSlot::request_heap_ref(handle, tag, flags),
                ValueSlot::as_request_heap_ref,
            ),
            (
                ValueKind::ActorStateRef,
                ValueSlot::actor_state_ref(handle, tag, flags),
                ValueSlot::as_actor_state_ref,
            ),
            (
                ValueKind::ConstRef,
                ValueSlot::const_ref(handle, tag, flags),
                ValueSlot::as_const_ref,
            ),
            (
                ValueKind::ResourceRef,
                ValueSlot::resource_ref(handle, tag, flags),
                ValueSlot::as_resource_ref,
            ),
            (
                ValueKind::CallbackClosureRef,
                ValueSlot::callback_closure_ref(handle, tag, flags),
                ValueSlot::as_callback_closure_ref,
            ),
        ];

        for (kind, slot, read_specific_handle) in cases {
            assert_eq!(slot.kind(), Some(kind));
            assert_eq!(slot.as_handle(), Some(handle));
            assert_eq!(read_specific_handle(&slot), Some(handle));
            assert_eq!(slot.compact_type_tag(), Some(tag));
            assert_eq!(slot.flags(), flags);
        }
    }

    #[test]
    fn type_index_zero_and_largest_encodable_index_round_trip_without_a_sentinel() {
        for type_index in [0, u32::MAX - 1] {
            let tag = tag(type_index);
            assert_eq!(tag.type_index(), type_index);
            let slot = ValueSlot::request_heap_ref(VmHandle::new(7), tag, ValueFlags::new(u8::MAX));
            assert_eq!(slot.kind(), Some(ValueKind::RequestHeapRef));
            assert_eq!(slot.compact_type_tag(), Some(tag));
            assert_eq!(
                slot.compact_type_tag().map(CompactTypeTag::type_index),
                Some(type_index)
            );
        }

        assert_eq!(CompactTypeTag::try_from_type_index(u32::MAX), None);
    }

    #[test]
    fn mismatched_reads_fail_closed() {
        let number = ValueSlot::number(7.0);
        let request_ref =
            ValueSlot::request_heap_ref(VmHandle::new(9), tag(10), ValueFlags::new(11));

        assert_eq!(number.as_bool(), None);
        assert_eq!(number.as_integer(), None);
        assert_eq!(number.as_handle(), None);
        assert_eq!(request_ref.as_number(), None);
        assert_eq!(request_ref.as_actor_state_ref(), None);

        let invalid_bool =
            ValueSlot::from_raw_parts_for_test(2, ValueSlot::bool(false).metadata_for_test());
        assert_eq!(invalid_bool.as_bool(), None);

        let invalid_kind = ValueSlot::from_raw_parts_for_test(0, u64::from(u8::MAX));
        assert_eq!(invalid_kind.kind(), None);
        assert_eq!(invalid_kind.as_handle(), None);

        let reference_without_tag =
            ValueSlot::from_raw_parts_for_test(9, request_ref.metadata_for_test() & !TYPE_TAG_MASK);
        assert_eq!(reference_without_tag.compact_type_tag(), None);
        assert_eq!(reference_without_tag.kind(), None);
        assert_eq!(reference_without_tag.as_request_heap_ref(), None);

        let immediate_with_tag = ValueSlot::from_raw_parts_for_test(
            ValueSlot::integer(3).payload,
            ValueSlot::integer(3).metadata_for_test() | (1 << super::TYPE_TAG_SHIFT),
        );
        assert_eq!(immediate_with_tag.compact_type_tag(), Some(tag(0)));
        assert_eq!(immediate_with_tag.kind(), None);
        assert_eq!(immediate_with_tag.as_integer(), None);
    }

    #[test]
    fn constructors_clear_reserved_metadata_bits() {
        let slots = [
            ValueSlot::null(),
            ValueSlot::bool(true),
            ValueSlot::number(1.0),
            ValueSlot::integer(-1),
            ValueSlot::date(1),
            ValueSlot::request_heap_ref(
                VmHandle::new(u64::MAX),
                tag(u32::MAX - 1),
                ValueFlags::new(u8::MAX),
            ),
            ValueSlot::actor_state_ref(
                VmHandle::new(u64::MAX),
                tag(u32::MAX - 1),
                ValueFlags::new(u8::MAX),
            ),
            ValueSlot::const_ref(
                VmHandle::new(u64::MAX),
                tag(u32::MAX - 1),
                ValueFlags::new(u8::MAX),
            ),
            ValueSlot::resource_ref(
                VmHandle::new(u64::MAX),
                tag(u32::MAX - 1),
                ValueFlags::new(u8::MAX),
            ),
            ValueSlot::callback_closure_ref(
                VmHandle::new(u64::MAX),
                tag(u32::MAX - 1),
                ValueFlags::new(u8::MAX),
            ),
        ];

        for slot in slots {
            assert_eq!(slot.metadata_for_test() & RESERVED_MASK, 0);
        }

        let corrupt = ValueSlot::from_raw_parts_for_test(
            0,
            ValueSlot::null().metadata_for_test() | RESERVED_MASK,
        );
        assert_eq!(corrupt.kind(), None);
        assert_eq!(corrupt.as_null(), None);
    }
}
