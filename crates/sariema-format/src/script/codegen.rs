// SPDX-License-Identifier: Apache-2.0

// Macros that turn the batch table into types, codecs and descriptions.
// Shared verbatim by `sariema-format` and the no_std `sariema-sdk`, so the
// expansion uses only `core` and names that the invoking module imports.

macro_rules! wire_enum {
    (
        $(#[doc = $doc:literal])*
        $Name:ident {
            $( $(#[doc = $vdoc:literal])* $Variant:ident = $value:literal => $text:literal ),* $(,)?
        }
    ) => {
        $(#[doc = $doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[repr(u8)]
        pub enum $Name {
            $( $(#[doc = $vdoc])* $Variant = $value ),*
        }

        impl $Name {
            pub const SPEC: EnumSpec = EnumSpec {
                name: stringify!($Name),
                doc: concat!($($doc, "\n"),*),
                variants: &[ $( VariantSpec {
                    value: $value,
                    name: $text,
                    doc: concat!($($vdoc),*),
                } ),* ],
            };

            pub const fn from_u8(value: u8) -> Option<$Name> {
                match value {
                    $( $value => Some($Name::$Variant), )*
                    _ => None,
                }
            }

            pub fn from_name(name: &str) -> Option<$Name> {
                match name {
                    $( $text => Some($Name::$Variant), )*
                    _ => None,
                }
            }

            pub const fn name(self) -> &'static str {
                match self {
                    $( $Name::$Variant => $text, )*
                }
            }
        }

        impl Wire for $Name {
            type Ref<'a> = $Name;
            const NAME: &'static str = stringify!($Name);

            fn read<'a>(r: &mut Reader<'a>) -> Result<$Name, WireError> {
                let [value] = r.take::<1>()?;
                $Name::from_u8(value).ok_or(WireError::Invalid(stringify!($Name)))
            }

            fn write(v: $Name, out: &mut impl Sink) {
                out.put(&[v as u8]);
            }

            fn size(_: $Name) -> usize {
                1
            }

            fn sample(n: u32) -> $Name {
                const ALL: &[$Name] = &[$($Name::$Variant),*];
                ALL[n as usize % ALL.len()]
            }
        }
    };
}

// The decoded type of a field: concrete, so messages stay covariant in 'a.
macro_rules! field_ref {
    (Str, $l:lifetime) => { &$l str };
    (Bytes, $l:lifetime) => { &$l [u8] };
    (Vec3, $l:lifetime) => { [f32; 3] };
    ($t:ident, $l:lifetime) => { $t };
}

macro_rules! message_enum {
    (
        $Enum:ident, $SPECS:ident,
        $(
            $(#[doc = $doc:literal])*
            $op:literal $Name:ident $snake:ident {
                $( $(#[doc = $fdoc:literal])* $f:ident : $ty:ident ),*
            }
        )*
    ) => {
        #[derive(Clone, Copy, Debug, PartialEq)]
        pub enum $Enum<'a> {
            $(
                $(#[doc = $doc])*
                $Name { $( $(#[doc = $fdoc])* $f: field_ref!($ty, 'a) ),* },
            )*
        }

        impl<'a> $Enum<'a> {
            /// Reads one record. On an error the reader's position is unspecified.
            pub fn read(r: &mut Reader<'a>) -> Result<$Enum<'a>, WireError> {
                let op = <u16 as Wire>::read(r)?;
                match op {
                    $( $op => Ok($Enum::$Name { $( $f: <$ty as Wire>::read(r)? ),* }), )*
                    other => Err(WireError::UnknownOpcode(other)),
                }
            }

            pub fn write(&self, out: &mut impl Sink) {
                match *self {
                    $( $Enum::$Name { $( $f ),* } => {
                        <u16 as Wire>::write($op, out);
                        $( <$ty as Wire>::write($f, out); )*
                    } )*
                }
            }

            /// Bytes [`Self::write`] writes.
            pub fn size(&self) -> usize {
                match *self {
                    $( $Enum::$Name { $( $f ),* } => 2 $( + <$ty as Wire>::size($f) )*, )*
                }
            }

            pub const fn opcode(&self) -> u16 {
                match self {
                    $( $Enum::$Name { .. } => $op, )*
                }
            }

            pub const fn name(&self) -> &'static str {
                match self {
                    $( $Enum::$Name { .. } => stringify!($snake), )*
                }
            }

            /// One value of every message, with fields derived from the
            /// opcode, for round-trip tests generated from the table.
            #[doc(hidden)]
            #[allow(unused_mut, unused_variables, unused_assignments)]
            pub fn samples() -> impl Iterator<Item = $Enum<'static>> {
                [ $( {
                    let mut n = $op as u32;
                    $Enum::$Name { $( $f: {
                        n = n.wrapping_mul(31).wrapping_add(7);
                        <$ty as Wire>::sample(n)
                    } ),* }
                } ),* ].into_iter()
            }
        }

        pub const $SPECS: &[MessageSpec] = &[ $( MessageSpec {
            opcode: $op,
            name: stringify!($snake),
            doc: concat!($($doc, "\n"),*),
            fields: &[ $( FieldSpec {
                name: stringify!($f),
                ty: <$ty as Wire>::NAME,
                doc: concat!($($fdoc),*),
            } ),* ],
        } ),* ];
    };
}

macro_rules! batch_table {
    (
        version $version:literal;
        enums [ $($E:ident),* $(,)? ]
        commands { $($commands:tt)* }
        events { $($events:tt)* }
    ) => {
        /// The batch version this table describes. A game pack declares the
        /// version its logic was built against; any other is refused at load.
        pub const BATCH_VERSION: u32 = $version;

        pub const ENUMS: &[EnumSpec] = &[ $($E::SPEC),* ];

        message_enum!(Command, COMMANDS, $($commands)*);
        message_enum!(Event, EVENTS, $($events)*);
    };
}
