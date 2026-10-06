//! Owner arbitration (`V/motion-governor.js:131-164`, `00` §6.3): one owner animates at a time.
//!
//! Interactions raise a *flag* (the JS reads attributes on the `<svg>`); the derived owner is the
//! highest-priority flag. An explicit [`Owners::claim`] (story playback, a route journey) beats
//! every flag, preempts the previous claimant and returns a monotonically increasing token.
//! [`Owners::release`] is token-checked and idempotent. The JS runs the preempted claimant's
//! cleanup callback; here [`Claim::preempted`] names who was displaced and the caller cleans up.

use serde::{Deserialize, Serialize};

/// Who owns the motion budget. Declared in priority order, strongest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Owner {
    Route,
    Lens,
    Relationship,
    Intent,
    Focus,
    Legend,
    /// Explicit claims only (guided views); not a derived flag.
    Story,
}

impl Owner {
    /// The derived owners, strongest first.
    pub const DERIVED: [Owner; 6] =
        [Owner::Route, Owner::Lens, Owner::Relationship, Owner::Intent, Owner::Focus, Owner::Legend];

    fn flag(self) -> usize {
        self as usize
    }
}

/// A claim receipt. `token == 0` means nothing was claimed (the governor is not capable).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Token(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Claim {
    pub token: Token,
    /// The explicit claimant this claim displaced; its cleanup is the caller's.
    pub preempted: Option<Owner>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Owners {
    flags: [bool; 7],
    explicit: Option<Owner>,
    token: u64,
}

impl Owners {
    /// Raise or drop an interaction flag. Returns whether the effective owner changed.
    pub fn set_active(&mut self, owner: Owner, on: bool) -> bool {
        let before = self.owner();
        self.flags[owner.flag()] = on;
        before != self.owner()
    }

    pub fn is_active(&self, owner: Owner) -> bool {
        self.flags[owner.flag()]
    }

    /// `explicitOwner || deriveOwner()`.
    pub fn owner(&self) -> Option<Owner> {
        self.explicit.or_else(|| Owner::DERIVED.into_iter().find(|o| self.flags[o.flag()]))
    }

    /// Preempt whatever is claimed and take the budget.
    pub fn claim(&mut self, next: Owner) -> Claim {
        let preempted = self.explicit.take();
        self.token += 1;
        self.explicit = Some(next);
        Claim { token: Token(self.token), preempted }
    }

    /// Give the claim back. Stale or repeated releases do nothing and return `false`.
    pub fn release(&mut self, token: Token) -> bool {
        if token.0 != self.token || self.explicit.is_none() {
            return false;
        }
        self.explicit = None;
        self.token += 1;
        true
    }

    pub fn explicit(&self) -> Option<Owner> {
        self.explicit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_is_route_lens_relationship_intent_focus_legend() {
        let mut o = Owners::default();
        assert_eq!(o.owner(), None);
        for w in Owner::DERIVED.windows(2).rev() {
            o.set_active(w[1], true);
            o.set_active(w[0], true);
            assert_eq!(o.owner(), Some(w[0]), "{:?} over {:?}", w[0], w[1]);
        }
        o.set_active(Owner::Route, false);
        assert_eq!(o.owner(), Some(Owner::Lens));
        for p in Owner::DERIVED {
            o.set_active(p, false);
        }
        assert_eq!(o.owner(), None);
    }

    #[test]
    fn an_explicit_claim_beats_every_flag_and_preempts_the_last() {
        let mut o = Owners::default();
        o.set_active(Owner::Route, true);
        let a = o.claim(Owner::Story);
        assert_eq!(o.owner(), Some(Owner::Story));
        assert_eq!(a.preempted, None);
        let b = o.claim(Owner::Route);
        assert_eq!(b.preempted, Some(Owner::Story));
        assert!(b.token > a.token);
        // The displaced claimant can no longer release the new claim.
        assert!(!o.release(a.token));
        assert_eq!(o.owner(), Some(Owner::Route));
    }

    #[test]
    fn release_is_token_checked_and_idempotent() {
        let mut o = Owners::default();
        o.set_active(Owner::Focus, true);
        let c = o.claim(Owner::Story);
        assert!(o.release(c.token));
        assert_eq!(o.owner(), Some(Owner::Focus), "the flag owns again");
        assert!(!o.release(c.token), "second release is a no-op");
        let d = o.claim(Owner::Story);
        assert!(d.token > c.token, "tokens never repeat");
    }
}
