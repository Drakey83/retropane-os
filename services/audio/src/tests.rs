//! The S1 core against a model of the X51v's audio lines: a clock that
//! moves only when the service waits, the codec answering only while
//! powered, I2S dropping a switch-on within 645 ms of a switch-off (as
//! measured), the jack valid only with the codec set up, and an edge latch.

use std::cell::RefCell;
use std::rc::Rc;

use super::*;

/// What happened, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Ev {
    Power(bool),
    Amp(bool),
    PinsUp,
    PinsDown,
    LinkOn(bool),
    LinkOff,
    /// A codec register write: (register, value).
    Reg(u8, u16),
    JackRead,
}

#[derive(Default)]
struct World {
    us: u64,
    ev: Vec<Ev>,
    powered: bool,
    codec_present: bool,
    /// Writes fail from this many codec writes on (None: never).
    fail_after: Option<usize>,
    writes: usize,
    link_on: bool,
    link_off_at: Option<u64>,
    plugged: bool,
    edge: bool,
    /// When the codec's rate was set (its set-up), and the jack last read.
    set_up_at: Option<u64>,
    jack_read_at: Option<u64>,
}

type W = Rc<RefCell<World>>;

struct FakeBoard(W);
struct FakeLink(W);
struct FakeBus(W);

impl Board for FakeBoard {
    fn codec_power(&mut self, on: bool) {
        let mut w = self.0.borrow_mut();
        // The jack line is high only while powered and plugged: switching
        // the supply moves it, and the latch catches that edge.
        let before = w.powered && w.plugged;
        w.powered = on;
        w.edge |= before != (on && w.plugged);
        w.ev.push(Ev::Power(on));
    }
    fn speaker_amp(&mut self, on: bool) {
        self.0.borrow_mut().ev.push(Ev::Amp(on));
    }
    fn pins_up(&mut self) {
        self.0.borrow_mut().ev.push(Ev::PinsUp);
    }
    fn pins_down(&mut self) {
        self.0.borrow_mut().ev.push(Ev::PinsDown);
    }
    fn jack_in(&mut self) -> bool {
        let mut w = self.0.borrow_mut();
        w.ev.push(Ev::JackRead);
        w.jack_read_at = Some(w.us);
        // Valid only with the codec powered (and set up): else reads low.
        w.powered && w.plugged
    }
    fn take_jack_edge(&mut self) -> bool {
        core::mem::take(&mut self.0.borrow_mut().edge)
    }
    fn now_ms(&mut self) -> u32 {
        (self.0.borrow().us / 1_000) as u32
    }
    fn delay_us(&mut self, us: u32) {
        self.0.borrow_mut().us += u64::from(us);
    }
}

impl Link for FakeLink {
    fn start(&mut self) -> bool {
        let mut w = self.0.borrow_mut();
        // The X51v's rule: dropped within ~645 ms of a switch-off.
        let taken = w.link_off_at.is_none_or(|t| w.us - t >= 645_000);
        w.link_on = taken;
        w.ev.push(Ev::LinkOn(taken));
        taken
    }
    fn stop(&mut self) {
        let mut w = self.0.borrow_mut();
        if w.link_on {
            w.link_off_at = Some(w.us);
        }
        w.link_on = false;
        w.ev.push(Ev::LinkOff);
    }
}

impl I2cBus for FakeBus {
    fn write(&mut self, _addr: u8, data: &[u8]) -> Result<(), I2cError> {
        let mut w = self.0.borrow_mut();
        // No supply or no master clock: no answer.
        if !(w.powered && w.codec_present && w.link_on) {
            return Err(I2cError::NoDevice);
        }
        if w.fail_after.is_some_and(|n| w.writes >= n) {
            return Err(I2cError::Nak);
        }
        w.writes += 1;
        if data[0] >> 1 == 0x08 {
            w.set_up_at = Some(w.us);
        }
        let [a, b] = data else {
            panic!("codec writes are two bytes")
        };
        w.ev.push(Ev::Reg(a >> 1, u16::from(a & 1) << 8 | u16::from(*b)));
        Ok(())
    }
}

fn world(plugged: bool) -> W {
    Rc::new(RefCell::new(World {
        codec_present: true,
        plugged,
        ..World::default()
    }))
}

fn audio(w: &W) -> Audio<FakeBoard, FakeBus, FakeLink> {
    Audio::new(
        FakeBoard(w.clone()),
        FakeLink(w.clone()),
        FakeBus(w.clone()),
        70,
    )
}

fn pos(ev: &[Ev], e: &Ev) -> usize {
    ev.iter()
        .position(|x| x == e)
        .unwrap_or_else(|| panic!("{e:?} never happened: {ev:?}"))
}

#[test]
fn up_powers_then_clocks_then_talks_then_reads_the_jack() {
    let w = world(false);
    let mut a = audio(&w);
    let p = a.up().unwrap();
    assert_eq!(p.output, Output::Speaker);
    assert_eq!(p.waited_ms, 0);
    assert_eq!(p.address, 0x1A);
    let ev = w.borrow().ev.clone();
    let power = pos(&ev, &Ev::Power(true));
    let pins = pos(&ev, &Ev::PinsUp);
    let link = pos(&ev, &Ev::LinkOn(true));
    let reset = pos(&ev, &Ev::Reg(0x0F, 0));
    let srate = pos(&ev, &Ev::Reg(0x08, 0x020));
    let jack = pos(&ev, &Ev::JackRead);
    let amp = pos(&ev, &Ev::Amp(true));
    assert!(power < pins && pins < link && link < reset, "{ev:?}");
    assert!(reset < srate && srate < jack && jack < amp, "{ev:?}");
    // Muted through set-up; the level comes only after the route.
    let unmute = ev
        .iter()
        .rposition(|e| matches!(e, Ev::Reg(0x29, v) if v & 0x7F != 0))
        .unwrap();
    assert!(amp < unmute);
    assert!(a.is_up());
}

#[test]
fn the_jack_is_read_only_once_the_set_up_has_settled() {
    let w = world(true);
    let mut a = audio(&w);
    a.up().unwrap();
    let ww = w.borrow();
    let settled = ww.jack_read_at.unwrap() - ww.set_up_at.unwrap();
    assert!(
        settled >= u64::from(JACK_SETTLE_US),
        "read {settled} us after set-up"
    );
}

#[test]
fn vmid_charges_fast_and_is_on_playback_before_the_jack_read() {
    let w = world(true);
    let mut a = audio(&w);
    a.up().unwrap();
    let ev = w.borrow().ev.clone();
    let fast = pos(&ev, &Ev::Reg(0x19, 0x1C0));
    let normal = pos(&ev, &Ev::Reg(0x19, 0x0C0));
    let jack = pos(&ev, &Ev::JackRead);
    assert!(fast < normal && normal < jack, "{ev:?}");
}

#[test]
fn headphones_in_leave_the_amplifier_off() {
    let w = world(true);
    let mut a = audio(&w);
    assert_eq!(a.up().unwrap().output, Output::Headphones);
    assert!(!w.borrow().ev.contains(&Ev::Amp(true)));
}

#[test]
fn up_twice_does_nothing_the_second_time() {
    let w = world(false);
    let mut a = audio(&w);
    a.up().unwrap();
    let n = w.borrow().ev.len();
    assert_eq!(a.up().unwrap().output, Output::Speaker);
    assert_eq!(w.borrow().ev.len(), n);
}

#[test]
fn down_mutes_before_the_amplifier_and_the_supply_goes_last() {
    let w = world(false);
    let mut a = audio(&w);
    a.up().unwrap();
    let from = w.borrow().ev.len();
    a.down().unwrap();
    let ev = w.borrow().ev[from..].to_vec();
    assert_eq!(ev[0], Ev::Amp(false));
    let pwr1 = pos(&ev, &Ev::Reg(0x19, 0));
    let link = pos(&ev, &Ev::LinkOff);
    let pins = pos(&ev, &Ev::PinsDown);
    assert!(pwr1 < link && link < pins);
    assert_eq!(ev.last(), Some(&Ev::Power(false)));
    assert!(!a.is_up());
    // Down twice: nothing more.
    let n = w.borrow().ev.len();
    a.down().unwrap();
    assert_eq!(w.borrow().ev.len(), n);
}

#[test]
fn up_soon_after_down_waits_out_the_guard_and_is_taken() {
    let w = world(false);
    let mut a = audio(&w);
    a.up().unwrap();
    a.down().unwrap();
    w.borrow_mut().us += 100_000; // 100 ms later
    let p = a.up().unwrap();
    assert_eq!(p.waited_ms, RESTART_GUARD_MS - 100);
    // Never a dropped switch-on.
    assert!(!w.borrow().ev.contains(&Ev::LinkOn(false)));
    // Long after: no wait.
    a.down().unwrap();
    w.borrow_mut().us += 5_000_000;
    assert_eq!(a.up().unwrap().waited_ms, 0);
}

#[test]
fn the_guard_covers_the_measured_window() {
    const { assert!(RESTART_GUARD_MS > 645) };
}

#[test]
fn a_jack_edge_while_down_powers_nothing_and_is_read_at_the_next_up() {
    let w = world(false);
    let mut a = audio(&w);
    {
        let mut ww = w.borrow_mut();
        ww.plugged = true;
        ww.edge = true;
    }
    assert_eq!(a.poll_jack(), None);
    assert!(a.jack_dirty());
    assert!(w.borrow().ev.is_empty(), "nothing switched for a jack edge");
    let p = a.up().unwrap();
    assert!(p.jack_was_dirty);
    assert_eq!(p.output, Output::Headphones);
    assert!(!a.jack_dirty());
}

#[test]
fn our_own_power_down_is_not_a_jack_change() {
    let w = world(true);
    let mut a = audio(&w);
    a.up().unwrap();
    a.down().unwrap();
    // The line fell with the supply; nothing changed at the jack.
    assert_eq!(a.poll_jack(), None);
    assert!(!a.jack_dirty());
}

#[test]
fn a_jack_edge_while_up_reroutes_after_the_debounce() {
    let w = world(true);
    let mut a = audio(&w);
    a.up().unwrap();
    {
        let mut ww = w.borrow_mut();
        ww.plugged = false;
        ww.edge = true;
    }
    assert_eq!(a.poll_jack(), None, "not before the debounce");
    w.borrow_mut().us += u64::from(JACK_DEBOUNCE_MS - 1) * 1_000;
    assert_eq!(a.poll_jack(), None);
    w.borrow_mut().us += 1_000;
    let from = w.borrow().ev.len();
    assert_eq!(a.poll_jack(), Some(Output::Speaker));
    let ev = w.borrow().ev[from..].to_vec();
    // Amplifier on and settled before the speaker pair is unmuted.
    let amp = pos(&ev, &Ev::Amp(true));
    let speaker = ev
        .iter()
        .position(|e| matches!(e, Ev::Reg(0x29, v) if v & 0x7F != 0))
        .unwrap();
    assert!(amp < speaker);
    // Back to headphones: the speaker muted before the amplifier goes off.
    {
        let mut ww = w.borrow_mut();
        ww.plugged = true;
        ww.edge = true;
    }
    a.poll_jack();
    w.borrow_mut().us += u64::from(JACK_DEBOUNCE_MS) * 1_000;
    let from = w.borrow().ev.len();
    assert_eq!(a.poll_jack(), Some(Output::Headphones));
    let ev = w.borrow().ev[from..].to_vec();
    let mute = pos(&ev, &Ev::Reg(0x29, 0x180));
    assert!(mute < pos(&ev, &Ev::Amp(false)));
}

#[test]
fn a_bounce_back_to_the_same_output_changes_nothing() {
    let w = world(true);
    let mut a = audio(&w);
    a.up().unwrap();
    w.borrow_mut().edge = true; // edge, but the level is back where it was
    a.poll_jack();
    w.borrow_mut().us += 100_000;
    let n = w.borrow().ev.len();
    assert_eq!(a.poll_jack(), None);
    assert_eq!(w.borrow().ev.len(), n + 1, "only the jack read");
}

#[test]
fn no_codec_puts_everything_back_down() {
    let w = world(false);
    w.borrow_mut().codec_present = false;
    let mut a = audio(&w);
    assert_eq!(a.up(), Err(AudioError::Codec(I2cError::NoDevice)));
    let ev = w.borrow().ev.clone();
    assert_eq!(ev.last(), Some(&Ev::Power(false)));
    assert!(ev.contains(&Ev::LinkOff) && ev.contains(&Ev::PinsDown));
    assert!(!a.is_up());
    // And it can try again later (the bus came back).
    w.borrow_mut().codec_present = true;
    w.borrow_mut().us += 1_000_000;
    assert!(a.up().is_ok());
}

#[test]
fn a_codec_failing_mid_set_up_leaves_nothing_on() {
    let w = world(false);
    w.borrow_mut().fail_after = Some(3);
    let mut a = audio(&w);
    assert_eq!(a.up(), Err(AudioError::Codec(I2cError::Nak)));
    let ev = w.borrow().ev.clone();
    assert_eq!(ev.last(), Some(&Ev::Power(false)));
    assert!(!ev.contains(&Ev::Amp(true)));
}

#[test]
fn a_route_failing_after_the_amplifier_takes_it_back_off() {
    let w = world(false);
    // Reset + seven set-up writes + the muted level (4) + VMID to playback
    // pass; the route fails.
    w.borrow_mut().fail_after = Some(13);
    let mut a = audio(&w);
    assert!(matches!(a.up(), Err(AudioError::Codec(_))));
    let ev = w.borrow().ev.clone();
    assert!(pos(&ev, &Ev::Amp(true)) < pos(&ev, &Ev::Amp(false)));
    assert_eq!(ev.last(), Some(&Ev::Power(false)));
}

#[test]
fn a_dropped_switch_on_is_reported_and_undone() {
    let w = world(false);
    let mut a = audio(&w);
    // I2S was switched off just now by someone else (the guard can't know).
    w.borrow_mut().link_off_at = Some(0);
    w.borrow_mut().us = 10_000;
    assert_eq!(a.up(), Err(AudioError::LinkRefused));
    assert_eq!(w.borrow().ev.last(), Some(&Ev::Power(false)));
    // The guard now covers it: the next try waits and is taken.
    let p = a.up().unwrap();
    assert!(p.waited_ms > 0);
}

#[test]
fn the_level_applies_when_up_and_is_kept_when_down() {
    let w = world(true);
    let mut a = audio(&w);
    a.set_level(150).unwrap();
    assert_eq!(a.level(), 100);
    assert!(w.borrow().ev.is_empty());
    a.up().unwrap();
    // 100% is 0 dB on the headphone pair.
    assert!(w.borrow().ev.contains(&Ev::Reg(0x03, 0x1F9)));
}

// The codec's sleep (S1b, ADR-0032 R5): I2S keeps running.

fn link_events(w: &W) -> (usize, usize) {
    let ev = w.borrow().ev.clone();
    let on = ev.iter().filter(|e| matches!(e, Ev::LinkOn(_))).count();
    let off = ev.iter().filter(|e| **e == Ev::LinkOff).count();
    (on, off)
}

#[test]
fn the_codec_sleeps_after_the_quiet_time_and_the_link_keeps_running() {
    let w = world(false);
    let mut a = audio(&w);
    a.up().unwrap();
    w.borrow_mut().us += u64::from(SLEEP_AFTER_MS - 1) * 1_000;
    assert!(!a.idle(), "not before the quiet time");
    assert!(a.is_up());
    w.borrow_mut().us += 1_000;
    let from = w.borrow().ev.len();
    assert!(a.idle());
    assert!(a.is_asleep() && !a.is_up());
    let ev = w.borrow().ev[from..].to_vec();
    // The amplifier off before the codec's insides, the supply last.
    assert_eq!(ev[0], Ev::Amp(false));
    assert_eq!(ev.last(), Some(&Ev::Power(false)));
    assert!(ev.contains(&Ev::PinsDown));
    // I2S was never switched off.
    assert_eq!(link_events(&w), (1, 0));
    assert!(w.borrow().link_on);
    // Idle again: nothing more.
    let n = w.borrow().ev.len();
    assert!(!a.idle());
    assert_eq!(w.borrow().ev.len(), n);
}

#[test]
fn waking_from_sleep_needs_no_guard_and_no_link_restart() {
    let w = world(true);
    let mut a = audio(&w);
    a.up().unwrap();
    a.sleep().unwrap();
    // Straight back: the 645 ms rule would drop a link restart now.
    let p = a.wake().unwrap();
    assert!(p.from_sleep);
    assert_eq!(p.waited_ms, 0);
    assert_eq!(p.output, Output::Headphones);
    assert_eq!(link_events(&w), (1, 0));
    // The codec was set up again (its reset written twice in all).
    let resets = w
        .borrow()
        .ev
        .iter()
        .filter(|e| **e == Ev::Reg(0x0F, 0))
        .count();
    assert_eq!(resets, 2);
}

#[test]
fn a_wake_or_a_sound_restarts_the_quiet_time() {
    let w = world(false);
    let mut a = audio(&w);
    a.up().unwrap();
    w.borrow_mut().us += 4_000_000;
    // A touch: the shell wakes audio (already up: only the time restarts).
    assert!(!a.wake().unwrap().from_sleep);
    w.borrow_mut().us += 4_000_000;
    assert!(!a.idle(), "8 s after up but only 4 s after the touch");
    w.borrow_mut().us += 1_000_000;
    assert!(a.idle());
}

#[test]
fn going_down_from_sleep_stops_the_link_and_arms_the_guard() {
    let w = world(false);
    let mut a = audio(&w);
    a.up().unwrap();
    a.sleep().unwrap();
    a.down().unwrap();
    assert!(!a.is_asleep() && !a.is_up());
    assert_eq!(link_events(&w), (1, 1));
    // Straight back up: the full start-up, after the guard.
    let p = a.up().unwrap();
    assert!(!p.from_sleep);
    assert!(p.waited_ms > 600);
    assert!(!w.borrow().ev.contains(&Ev::LinkOn(false)));
}

#[test]
fn our_own_sleep_is_not_a_jack_change() {
    let w = world(true);
    let mut a = audio(&w);
    a.up().unwrap();
    a.sleep().unwrap();
    assert_eq!(a.poll_jack(), None);
    assert!(!a.jack_dirty());
    // A real change while asleep only marks it.
    {
        let mut ww = w.borrow_mut();
        ww.plugged = false;
        ww.edge = true;
    }
    assert_eq!(a.poll_jack(), None);
    assert!(a.jack_dirty());
    let p = a.wake().unwrap();
    assert!(p.jack_was_dirty);
    assert_eq!(p.output, Output::Speaker);
}
