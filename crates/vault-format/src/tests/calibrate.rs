//! KDF calibration with a fake timer.

use std::time::Duration;

use crate::{KdfParams, Limits, calibrate, measure_kdf};

const MIB: u32 = 1024;

/// A fake machine where a derivation costs `per_unit` for every MiB x iteration.
fn fake_machine(per_unit: Duration) -> impl FnMut(KdfParams) -> Duration {
    move |params| per_unit * (params.m_kib / MIB) * params.t
}

#[test]
fn a_machine_in_the_target_range_keeps_the_default() {
    // 512 MiB x 3 = 1536 units -> 0.768 s.
    let chosen = calibrate(
        &Limits::PRODUCTION,
        fake_machine(Duration::from_micros(500)),
    );
    assert_eq!(chosen, KdfParams::DEFAULT);
}

#[test]
fn a_slow_machine_halves_memory_until_a_run_takes_at_most_one_second() {
    // 512 MiB x 3 at 2.5 ms per unit = 3.84 s -> 256 MiB 1.92 s -> 128 MiB 0.96 s.
    let chosen = calibrate(
        &Limits::PRODUCTION,
        fake_machine(Duration::from_micros(2500)),
    );
    assert_eq!(
        chosen,
        KdfParams {
            m_kib: 128 * MIB,
            t: 3,
            p: 4
        }
    );
}

#[test]
fn a_very_slow_machine_stops_at_the_memory_floor() {
    let chosen = calibrate(
        &Limits::PRODUCTION,
        fake_machine(Duration::from_millis(100)),
    );
    assert_eq!(
        chosen,
        KdfParams {
            m_kib: Limits::PRODUCTION.min.m_kib,
            t: 3,
            p: 4
        }
    );
}

#[test]
fn a_fast_machine_adds_iterations_until_a_run_takes_half_a_second() {
    // 512 MiB at 0.1 ms per unit: t=3 0.154 s ... t=9 0.461 s, t=10 0.512 s.
    let chosen = calibrate(
        &Limits::PRODUCTION,
        fake_machine(Duration::from_micros(100)),
    );
    assert_eq!(
        chosen,
        KdfParams {
            m_kib: 512 * MIB,
            t: 10,
            p: 4
        }
    );
}

#[test]
fn an_iteration_that_overshoots_one_second_is_taken_back() {
    let mut seen = Vec::new();
    let chosen = calibrate(&Limits::PRODUCTION, |params| {
        seen.push(params);
        if params.t <= 3 {
            Duration::from_millis(400)
        } else {
            Duration::from_millis(1200)
        }
    });
    assert_eq!(chosen, KdfParams::DEFAULT);
    // Cheapest first: memory doubles from the floor to the default, then one iteration is tried.
    let at = |m_kib, t| KdfParams { m_kib, t, p: 4 };
    assert_eq!(
        seen,
        vec![
            at(64 * MIB, 3),
            at(128 * MIB, 3),
            at(256 * MIB, 3),
            at(512 * MIB, 3),
            at(512 * MIB, 4)
        ]
    );
}

#[test]
fn an_impossibly_fast_machine_stops_at_the_iteration_ceiling() {
    let chosen = calibrate(&Limits::PRODUCTION, |_| Duration::ZERO);
    assert_eq!(
        chosen,
        KdfParams {
            t: Limits::PRODUCTION.max.t,
            ..KdfParams::DEFAULT
        }
    );
}

#[test]
fn calibration_starts_at_the_floor_of_narrower_limits() {
    let limits = Limits {
        min: KdfParams {
            m_kib: 16 * MIB,
            t: 4,
            p: 1,
        },
        max: KdfParams {
            m_kib: 128 * MIB,
            t: 6,
            p: 2,
        },
    };
    // p and t clamped into the limits; memory starts at their floor and a 0.7 s run is kept.
    let clamped = KdfParams {
        m_kib: 16 * MIB,
        t: 4,
        p: 2,
    };
    let mut first = None;
    let chosen = calibrate(&limits, |params| {
        first.get_or_insert(params);
        Duration::from_millis(700)
    });
    assert_eq!(first, Some(clamped));
    assert_eq!(chosen, clamped);
    assert_eq!(limits.check(chosen), Ok(()));
}

#[test]
fn every_measured_candidate_is_within_the_limits() {
    for per_unit in [0u64, 1, 10, 100, 500, 1000, 2500, 10_000, 1_000_000] {
        let calibrated = calibrate(&Limits::PRODUCTION, |params| {
            assert_eq!(Limits::PRODUCTION.check(params), Ok(()), "{params:?}");
            Duration::from_micros(per_unit) * (params.m_kib / MIB) * params.t
        });
        assert_eq!(Limits::PRODUCTION.check(calibrated), Ok(()));
    }
}

#[test]
fn measure_kdf_times_a_real_derivation() {
    let elapsed = measure_kdf(KdfParams::TESTING);
    assert!(elapsed < Duration::from_secs(10));
}

#[test]
fn a_memory_starved_machine_measures_cheap_settings_first() {
    // Every doubling of memory costs 8x (swapping): the expensive default is never tried.
    let mut seen = Vec::new();
    let chosen = calibrate(&Limits::PRODUCTION, |params| {
        seen.push(params);
        let doublings = (params.m_kib / (64 * MIB)).trailing_zeros();
        Duration::from_millis(400) * 8u32.pow(doublings) * params.t / 3
    });
    // 64 MiB 0.4 s -> 128 MiB 3.2 s (not kept) -> t=4 at 64 MiB 0.53 s (kept).
    let at = |m_kib, t| KdfParams { m_kib, t, p: 4 };
    assert_eq!(chosen, at(64 * MIB, 4));
    assert_eq!(
        seen,
        vec![at(64 * MIB, 3), at(128 * MIB, 3), at(64 * MIB, 4)]
    );
}
