//! Good identity and per-world role assignment.
//!
//! A world has four elements (slots 0..4). Each element yields tradeable
//! *items*, indexed by `slot * 2 + form` (`Raw` = 0, `Refined` = 1), so there are
//! up to 8 item slots. Of these, four are the world's **consumable goods**: each
//! chosen element is assigned one role pairing a category (Staple/Positional)
//! with a form (Raw=unrefined / Refined). Exactly one of each combination:
//!
//! - staple · unrefined      - positional · unrefined
//! - staple · refined        - positional · refined
//!
//! When a good's form is `Refined`, its `Raw` item is an *intermediate* that a
//! refiner must convert before anyone can consume it.

use crate::rng::Rng;

pub const N_ITEMS: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GoodForm {
    Raw = 0,
    Refined = 1,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GoodCategory {
    Staple,
    Positional,
}

/// What an item slot means to a noot.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ItemRole {
    /// Directly edible staple; carries its hunger sub-index (0..2).
    Staple(usize),
    /// Directly consumable positional good; carries its sub-index (0..2).
    Positional(usize),
    /// Raw input that must be refined before it can be consumed.
    Intermediate,
    /// Not used by this world.
    Junk,
}

pub fn item_index(slot: usize, form: GoodForm) -> usize {
    slot * 2 + form as usize
}

pub fn form_of(item: usize) -> GoodForm {
    if item % 2 == 0 {
        GoodForm::Raw
    } else {
        GoodForm::Refined
    }
}

#[derive(Clone, Copy)]
pub struct ConsumableGood {
    pub slot: usize,
    pub form: GoodForm,
    pub category: GoodCategory,
    /// Index within its category (0 or 1).
    pub sub: usize,
}

pub struct WorldGoods {
    pub goods: [ConsumableGood; 4],
    pub item_roles: [ItemRole; N_ITEMS],
}

impl WorldGoods {
    pub fn role_of(&self, item: usize) -> ItemRole {
        self.item_roles[item]
    }

    /// Item index a noot must hold to consume the given consumable good.
    pub fn consumable_item(&self, good: &ConsumableGood) -> usize {
        item_index(good.slot, good.form)
    }
}

/// Randomly assign the four role pairings to the four element slots.
pub fn assign(rng: &mut Rng) -> WorldGoods {
    let mut roles = [
        (GoodCategory::Staple, GoodForm::Raw),
        (GoodCategory::Staple, GoodForm::Refined),
        (GoodCategory::Positional, GoodForm::Raw),
        (GoodCategory::Positional, GoodForm::Refined),
    ];
    rng.shuffle(&mut roles);

    let mut item_roles = [ItemRole::Junk; N_ITEMS];
    let mut staple_sub = 0usize;
    let mut positional_sub = 0usize;

    let goods = core::array::from_fn(|slot| {
        let (category, form) = roles[slot];
        let sub = match category {
            GoodCategory::Staple => {
                let s = staple_sub;
                staple_sub += 1;
                s
            }
            GoodCategory::Positional => {
                let s = positional_sub;
                positional_sub += 1;
                s
            }
        };

        let consumable = item_index(slot, form);
        item_roles[consumable] = match category {
            GoodCategory::Staple => ItemRole::Staple(sub),
            GoodCategory::Positional => ItemRole::Positional(sub),
        };
        if form == GoodForm::Refined {
            item_roles[item_index(slot, GoodForm::Raw)] = ItemRole::Intermediate;
        }

        ConsumableGood {
            slot,
            form,
            category,
            sub,
        }
    });

    WorldGoods { goods, item_roles }
}
