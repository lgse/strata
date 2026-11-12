// SPDX-License-Identifier: MIT

//! Every shortcut as a named constant, grouped by area. Handlers match these
//! constants, and the F1 reference, hints and labels show the same ones.

use super::{Binding, Command, Digits, Letters};

/// Declares an area's constants and lists them for the registry and the
/// catalog tests. The F1 reference shows `listed` constants in both maps,
/// `default_only` and `tenxer_only` ones in that map alone, and `unlisted` ones
/// in neither.
///
/// A `Command` names its id, and `trigger(Group)` or `step(Group)` for a
/// chord key or `structural` for a key read through aliases. `= binding` binds
/// one key and `=> [first, second]` alternates, primary first, in the maps its
/// block lists it in; `live in both` also binds it in the other map, and an
/// unlisted command names its maps with `live in default`, `live in tenxer` or
/// `live in both`. A command whose maps bind different keys lists each,
/// `=> default [..], tenxer [..]`, whatever its block. A single command that
/// follows another names it with `after`.
macro_rules! catalog {
    (@after) => { None };
    (@after $prefix:path) => { Some($prefix) };

    (@kind) => { $crate::keys::command::Kind::Single };
    (@kind trigger($group:ident)) => {
        $crate::keys::command::Kind::Trigger($crate::keys::command::Group::$group)
    };
    (@kind step($group:ident)) => {
        $crate::keys::command::Kind::Step($crate::keys::command::Group::$group)
    };
    (@kind structural) => { $crate::keys::command::Kind::Structural };

    (@default Both; $($value:expr),+) => { &[$($value),+] };
    (@default Default; $($value:expr),+) => { &[$($value),+] };
    (@default Tenxer; $($value:expr),+) => { &[] };
    (@default $block:ident both; $($value:expr),+) => { &[$($value),+] };
    (@default Unlisted default; $($value:expr),+) => { &[$($value),+] };
    (@default Unlisted tenxer; $($value:expr),+) => { &[] };
    (@tenxer Both; $($value:expr),+) => { &[$($value),+] };
    (@tenxer Default; $($value:expr),+) => { &[] };
    (@tenxer Tenxer; $($value:expr),+) => { &[$($value),+] };
    (@tenxer $block:ident both; $($value:expr),+) => { &[$($value),+] };
    (@tenxer Unlisted default; $($value:expr),+) => { &[] };
    (@tenxer Unlisted tenxer; $($value:expr),+) => { &[$($value),+] };

    (@define [$($attr:tt)*] $name:ident Binding [] [$value:expr] [] $block:ident []) => {
        $($attr)* pub const $name: Binding = $value;
    };
    (@define [$($attr:tt)*] $name:ident Digits [] [$value:expr] [] $block:ident []) => {
        $($attr)* pub const $name: Digits = $value;
    };
    (@define [$($attr:tt)*] $name:ident Letters [] [$value:expr] [] $block:ident []) => {
        $($attr)* pub const $name: Letters = $value;
    };
    (
        @define [$($attr:tt)*] $name:ident Command [$id:literal $($kind:ident $(($group:ident))?)?]
            [$($value:expr),+] [] $block:ident [$($live:ident)?]
    ) => {
        $($attr)* pub const $name: Command =
            Command::new(&$crate::keys::command::Spec {
                id: $id,
                kind: catalog!(@kind $($kind $(($group))?)?),
                default: catalog!(@default $block $($live)?; $($value),+),
                tenxer: catalog!(@tenxer $block $($live)?; $($value),+),
            });
    };
    (
        @define [$($attr:tt)*] $name:ident Command [$id:literal $($kind:ident $(($group:ident))?)?]
            [] [[$($default:expr),*] [$($tenxer:expr),*]] $block:ident []
    ) => {
        $($attr)* pub const $name: Command =
            Command::new(&$crate::keys::command::Spec {
                id: $id,
                kind: catalog!(@kind $($kind $(($group))?)?),
                default: &[$($default),*],
                tenxer: &[$($tenxer),*],
            });
    };

    (
        $(listed {
            $($(#[$meta:meta])* $name:ident: $kind:ident
                $($id:literal $($ck:ident $(($cg:ident))?)?)?
                $(= $value:expr)?
                $(=> [$($any:expr),+ $(,)?])?
                $(=> default [$($dv:expr),* $(,)?], tenxer [$($tv:expr),* $(,)?])?
                $(, after $prefix:path)? $(, live in $live:ident)?;)*
        })?
        $(default_only {
            $($(#[$dmeta:meta])* $dname:ident: $dkind:ident
                $($did:literal $($dck:ident $(($dcg:ident))?)?)?
                $(= $dvalue:expr)?
                $(=> [$($dany:expr),+ $(,)?])?
                $(=> default [$($ddv:expr),* $(,)?], tenxer [$($dtv:expr),* $(,)?])?
                $(, after $dprefix:path)? $(, live in $dlive:ident)?;)*
        })?
        $(tenxer_only {
            $($(#[$tmeta:meta])* $tname:ident: $tkind:ident
                $($tid:literal $($tck:ident $(($tcg:ident))?)?)?
                $(= $tvalue:expr)?
                $(=> [$($tany:expr),+ $(,)?])?
                $(=> default [$($tdv:expr),* $(,)?], tenxer [$($ttv:expr),* $(,)?])?
                $(, after $tprefix:path)? $(, live in $tlive:ident)?;)*
        })?
        $(unlisted {
            $($(#[$umeta:meta])* $uname:ident: $ukind:ident
                $($uid:literal $($uck:ident $(($ucg:ident))?)?)?
                $(= $uvalue:expr)?
                $(=> [$($uany:expr),+ $(,)?])?
                $(=> default [$($udv:expr),* $(,)?], tenxer [$($utv:expr),* $(,)?])?
                $(, live in $ulive:ident)?;)*
        })?
    ) => {
        $($(catalog!(@define [$(#[$meta])*] $name $kind [$($id $($ck $(($cg))?)?)?]
            [$($value)? $($($any),+)?] [$([$($dv),*] [$($tv),*])?]
            Both [$($live)?]);)*)?
        $($(catalog!(@define [$(#[$dmeta])*] $dname $dkind [$($did $($dck $(($dcg))?)?)?]
            [$($dvalue)? $($($dany),+)?] [$([$($ddv),*] [$($dtv),*])?]
            Default [$($dlive)?]);)*)?
        $($(catalog!(@define [$(#[$tmeta])*] $tname $tkind [$($tid $($tck $(($tcg))?)?)?]
            [$($tvalue)? $($($tany),+)?] [$([$($tdv),*] [$($ttv),*])?]
            Tenxer [$($tlive)?]);)*)?
        $($(catalog!(@define [$(#[$umeta])*] $uname $ukind [$($uid $($uck $(($ucg))?)?)?]
            [$($uvalue)? $($($uany),+)?] [$([$($udv),*] [$($utv),*])?]
            Unlisted [$($ulive)?]);)*)?

        pub(super) const ENTRIES: &[super::Entry] = &[
            $($(super::Entry::$kind($name),)*)?
            $($(super::Entry::$dkind($dname),)*)?
            $($(super::Entry::$tkind($tname),)*)?
            $($(super::Entry::$ukind($uname),)*)?
        ];

        #[cfg(any(test, feature = "test-support"))]
        pub(super) const LISTED: &[super::Listed] = &[
            $($(super::Listed {
                name: stringify!($name),
                entry: super::Entry::$kind($name),
                after: catalog!(@after $($prefix)?),
                maps: super::Maps::Both,
            },)*)?
            $($(super::Listed {
                name: stringify!($dname),
                entry: super::Entry::$dkind($dname),
                after: catalog!(@after $($dprefix)?),
                maps: super::Maps::Default,
            },)*)?
            $($(super::Listed {
                name: stringify!($tname),
                entry: super::Entry::$tkind($tname),
                after: catalog!(@after $($tprefix)?),
                maps: super::Maps::Tenxer,
            },)*)?
        ];
        #[cfg(any(test, feature = "test-support"))]
        pub(super) const UNLISTED: &[(&str, super::Entry)] =
            &[$($((stringify!($uname), super::Entry::$ukind($uname)),)*)?];
    };
}

mod chooser;
mod dialogs;
mod navigation;
mod preview;
mod reference;
mod tenxer;
mod widgets;
mod window;

pub use chooser::*;
pub use dialogs::*;
pub use navigation::*;
pub use preview::*;
pub use reference::*;
pub use tenxer::*;
pub use widgets::*;
pub use window::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entry {
    Binding(Binding),
    Digits(Digits),
    Letters(Letters),
    Command(Command),
}

/// Every constant the catalog declares, listed or not.
pub fn entries() -> impl Iterator<Item = Entry> {
    [
        chooser::ENTRIES,
        dialogs::ENTRIES,
        navigation::ENTRIES,
        preview::ENTRIES,
        reference::ENTRIES,
        tenxer::ENTRIES,
        widgets::ENTRIES,
        window::ENTRIES,
    ]
    .into_iter()
    .flatten()
    .copied()
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Copy, Debug)]
pub struct Listed {
    pub name: &'static str,
    pub entry: Entry,
    /// The command this one follows, besides a chord step's trigger.
    pub after: Option<Command>,
    pub maps: Maps,
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Maps {
    Both,
    Default,
    Tenxer,
}

#[cfg(any(test, feature = "test-support"))]
impl Maps {
    pub fn include(self, tenxer: bool) -> bool {
        match self {
            Self::Both => true,
            Self::Default => !tenxer,
            Self::Tenxer => tenxer,
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn listed() -> impl Iterator<Item = Listed> {
    [
        chooser::LISTED,
        dialogs::LISTED,
        navigation::LISTED,
        preview::LISTED,
        reference::LISTED,
        tenxer::LISTED,
        widgets::LISTED,
        window::LISTED,
    ]
    .into_iter()
    .flatten()
    .copied()
}

#[cfg(any(test, feature = "test-support"))]
pub fn unlisted() -> impl Iterator<Item = (&'static str, Entry)> {
    [
        chooser::UNLISTED,
        dialogs::UNLISTED,
        navigation::UNLISTED,
        preview::UNLISTED,
        reference::UNLISTED,
        tenxer::UNLISTED,
        widgets::UNLISTED,
        window::UNLISTED,
    ]
    .into_iter()
    .flatten()
    .copied()
}
