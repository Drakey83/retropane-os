//! Stream roles (ADR-0032 R3, A2, the owner's condition 1): what a sound
//! is for, declared by a native program when it opens a stream; old
//! programs get theirs by a fixed mapping. The headphone unplug policy and
//! (S3) the levels are per role.

/// What a stream is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Music, video, games' sound.
    Media,
    /// Calls (later, Bluetooth).
    Voice,
    /// A program's notification sound.
    Notification,
    /// Alarms: must be heard.
    Alarm,
    /// The shell's own sounds (clicks), `PlaySound` and `MessageBeep`.
    System,
}

impl Role {
    /// Every role, in a fixed order (the index of [`Role::index`]).
    pub const ALL: [Self; 5] = [
        Self::Media,
        Self::Voice,
        Self::Notification,
        Self::Alarm,
        Self::System,
    ];

    /// Its place in [`Role::ALL`].
    #[must_use]
    pub fn index(self) -> usize {
        match self {
            Self::Media => 0,
            Self::Voice => 1,
            Self::Notification => 2,
            Self::Alarm => 3,
            Self::System => 4,
        }
    }

    /// Whether a stream of this role wants low latency by default (the
    /// shell's sounds and calls: ADR-0032 R2's hint, set for them).
    #[must_use]
    pub fn interactive_by_default(self) -> bool {
        matches!(self, Self::System | Self::Voice)
    }
}

/// The old Windows Mobile sound calls (the CE personality, R7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LegacyCall {
    /// `waveOutOpen` and the rest of `waveOut`.
    WaveOut,
    /// `PlaySound`.
    PlaySound,
    /// `sndPlaySound`.
    SndPlaySound,
    /// `MessageBeep`.
    MessageBeep,
    /// The clock's and calendar's alarms (the CE notification API's sounds).
    NotificationAlarm,
}

/// The documented legacy mapping (ADR-0032 R3): `waveOut` is Media;
/// `PlaySound`, `sndPlaySound` and `MessageBeep` are System; the
/// notification API's alarms are Alarm.
#[must_use]
pub fn legacy_role(call: LegacyCall) -> Role {
    match call {
        LegacyCall::WaveOut => Role::Media,
        LegacyCall::PlaySound | LegacyCall::SndPlaySound | LegacyCall::MessageBeep => Role::System,
        LegacyCall::NotificationAlarm => Role::Alarm,
    }
}

/// What a stream does when the headphones come out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unplug {
    /// Paused; the program gets a "paused by unplug" mark and resumes when
    /// the user says so.
    Pause,
    /// Muted until the headphones go back in (or the user unmutes it): an
    /// old program can't be paused from outside.
    Mute,
    /// Carries on, on the speaker.
    Speaker,
}

/// The unplug policy per role (ADR-0032 A2, the owner's vote A): media
/// pauses (an old program's media, which can't be paused, is muted
/// instead); voice, notifications, system sounds and alarms move to the
/// speaker.
#[must_use]
pub fn on_unplug(role: Role, legacy: bool) -> Unplug {
    match role {
        Role::Media if legacy => Unplug::Mute,
        Role::Media => Unplug::Pause,
        Role::Voice | Role::Notification | Role::Alarm | Role::System => Unplug::Speaker,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_legacy_mapping_is_the_documented_one() {
        assert_eq!(legacy_role(LegacyCall::WaveOut), Role::Media);
        assert_eq!(legacy_role(LegacyCall::PlaySound), Role::System);
        assert_eq!(legacy_role(LegacyCall::SndPlaySound), Role::System);
        assert_eq!(legacy_role(LegacyCall::MessageBeep), Role::System);
        assert_eq!(legacy_role(LegacyCall::NotificationAlarm), Role::Alarm);
    }

    #[test]
    fn unplugging_pauses_media_mutes_old_media_and_moves_the_rest() {
        assert_eq!(on_unplug(Role::Media, false), Unplug::Pause);
        assert_eq!(on_unplug(Role::Media, true), Unplug::Mute);
        for r in [Role::Voice, Role::Notification, Role::Alarm, Role::System] {
            assert_eq!(on_unplug(r, false), Unplug::Speaker, "{r:?}");
            assert_eq!(on_unplug(r, true), Unplug::Speaker, "{r:?}");
        }
    }

    #[test]
    fn indices_match_the_list() {
        for (i, r) in Role::ALL.into_iter().enumerate() {
            assert_eq!(r.index(), i);
        }
    }
}
