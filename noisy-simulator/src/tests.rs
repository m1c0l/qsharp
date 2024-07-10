// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use crate::{
    instrument::Instrument,
    operation::{operation, Operation},
    NoisySimulator, TOLERANCE,
};
use num_complex::Complex;

/// Assert that two f64 are equal up to a `TOLERANCE`.
pub fn assert_approx_eq(left: f64, right: f64) {
    if (left - right).abs() > TOLERANCE {
        panic!("aprox_equal failed, left = {left}, right = {right}");
    }
}

/// Returns an H gate.
fn h() -> Operation {
    let f = 0.5_f64.sqrt();
    operation!([f,  f;
                f, -f;])
}

/// Returns a CNOT gate.
fn cnot() -> Operation {
    operation!([1., 0., 0., 0.;
                0., 1., 0., 0.;
                0., 0., 0., 1.;
                0., 0., 1., 0.;])
}

/// Returns the 0-projection of an MZ measurement.
fn mz0() -> Operation {
    operation!([1., 0.;
                0., 0.;])
}

/// Returns the 1-projection of an MZ measurement.
fn mz1() -> Operation {
    operation!([0., 0.;
                0., 1.;])
}

/// Returns an MZ measurement.
fn mz() -> Instrument {
    Instrument::new(vec![mz0(), mz1()])
}

pub fn measure_0<NS: NoisySimulator>() {
    let h = h();
    let mz = mz();
    let mut sim = NS::new(1);
    sim.apply_operation(&h, &[0])
        .expect("operation should succeed");

    // Random samples less than 0.5 should yield a 0-measurement.
    let measurement = sim
        .sample_instrument_with_distribution(&mz, &[0], 0.49999)
        .expect("measurement should succeed");
    assert_eq!(measurement, 0);
}

pub fn measure_1<NS: NoisySimulator>() {
    let h = h();
    let mz = mz();
    let mut sim = NS::new(1);
    sim.apply_operation(&h, &[0])
        .expect("operation should succeed");

    // Random samples greater than 0.5 should yield a 1-measurement.
    let measurement = sim
        .sample_instrument_with_distribution(&mz, &[0], 0.50001)
        .expect("measurement should succeed");
    assert_eq!(measurement, 1);
}

/// Check that both measurements in a Bell Pair yield the same result.
pub fn bell_pair_sampling<NS: NoisySimulator>(seed: u64) {
    let (h, cnot, mz) = (h(), cnot(), mz());
    let mut sim = NS::new(2);
    sim.set_rng_seed(seed);

    // Make a Bell Pair.
    sim.apply_operation(&h, &[0])
        .expect("operation should succeed");
    sim.apply_operation(&cnot, &[1, 0])
        .expect("operation should succeed");

    // Measure both qubits.
    let m1 = sim
        .sample_instrument(&mz, &[0])
        .expect("measurement should succeed");
    let m2 = sim
        .sample_instrument(&mz, &[1])
        .expect("measurement should succeed");

    // Check that both measurements yield the same result.
    assert_eq!(m1, m2);
}

/// Project both qubits of a Bell Pair on the mz0 direction.
/// The trace of the system (i.e. the probability of finding
/// the quantum system in this state) should be 0.5.
pub fn bell_pair_projection_mz0<NS: NoisySimulator>() {
    let (h, cnot, mz0) = (h(), cnot(), mz0());
    let mut sim = NS::new(2);

    // Make a Bell Pair.
    sim.apply_operation(&h, &[0])
        .expect("operation should succeed");
    sim.apply_operation(&cnot, &[1, 0])
        .expect("operation should succeed");

    // Project both qubits on the mz0 direction.
    sim.apply_operation(&mz0, &[0])
        .expect("operation should succeed");
    sim.apply_operation(&mz0, &[1])
        .expect("operation should succeed");
    assert_approx_eq(0.5, sim.trace_change().expect("state should be valid"));

    // Repeating the projection twice should yield the same result.
    sim.apply_operation(&mz0, &[0])
        .expect("operation should succeed");
    sim.apply_operation(&mz0, &[1])
        .expect("operation should succeed");
    assert_approx_eq(0.5, sim.trace_change().expect("state should be valid"));
}

/// Project both qubits of a Bell Pair on the mz1 direction.
/// The trace of the system (i.e. the probability of finding
/// the quantum system in this state) should be 0.5.
pub fn bell_pair_projection_mz1<NS: NoisySimulator>() {
    let (h, cnot, mz1) = (h(), cnot(), mz1());
    let mut sim = NS::new(2);

    // Make a Bell Pair.
    sim.apply_operation(&h, &[0])
        .expect("operation should succeed");
    sim.apply_operation(&cnot, &[1, 0])
        .expect("operation should succeed");

    // Project both qubits on the mz1 direction.
    sim.apply_operation(&mz1, &[0])
        .expect("operation should succeed");
    sim.apply_operation(&mz1, &[1])
        .expect("operation should succeed");
    assert_approx_eq(0.5, sim.trace_change().expect("state should be valid"));

    // Repeating the projection twice should yield the same result.
    sim.apply_operation(&mz1, &[0])
        .expect("operation should succeed");
    sim.apply_operation(&mz1, &[1])
        .expect("operation should succeed");
    assert_approx_eq(0.5, sim.trace_change().expect("state should be valid"));
}

/// Project one qubit of a Bell Pair on the mz0 direction and the other on the mz1 direction.
/// This should yield a 0-probability error.
pub fn bell_pair_projection_oposite_directions<NS: NoisySimulator>() {
    let (h, cnot, mz0, mz1) = (h(), cnot(), mz0(), mz1());
    let mut sim = NS::new(2);

    // Make a Bell Pair.
    sim.apply_operation(&h, &[0])
        .expect("operation should succeed");
    sim.apply_operation(&cnot, &[1, 0])
        .expect("operation should succeed");

    // Project first qubit on the mz0 direction.
    sim.apply_operation(&mz0, &[0])
        .expect("operation should succeed");

    // Project second qubit on the mz1 direction.
    // This should yield a 0-probability error.
    sim.apply_operation(&mz1, &[1])
        .expect("operation should fail");
}

/// Check that projecting the target qubit in a CRX gate on the mz0 direction yields the right probabilities.
pub fn crx_gate_projection_mz0<NS: NoisySimulator>() {
    let (h, mz0, mz1) = (h(), mz0(), mz1());
    let probabilities: Vec<f64> = vec![0.05, 0.1, 0.3, 0.7, 0.8, 0.9, 0.99];

    // A CRX gate (Controlled Rotation around X axis).
    let crx = |t: f64| {
        let c = t.cos();
        let s = t.sin() * Complex::I;
        operation!([1., 0., 0., 0.;
                    0., 1., 0., 0.;
                    0., 0., c,  s;
                    0., 0., s,  c;])
    };

    for p in &probabilities {
        let t = p.sqrt().acos();
        let mut sim = NS::new(2);

        // Apply CRX gate
        sim.apply_operation(&h, &[0])
            .expect("operation should succeed");
        sim.apply_operation(&crx(0.3 * t), &[1, 0])
            .expect("operation should succeed");
        sim.apply_operation(&crx(0.7 * t), &[1, 0])
            .expect("operation should succeed");

        sim.apply_operation(&mz1, &[0])
            .expect("operation should succeed");
        assert_approx_eq(0.5, sim.trace_change().expect("state should be valid"));

        // Project target qubit on mz0 and check the trace
        // (the probability of finding the system in that state).
        sim.apply_operation(&mz0, &[1])
            .expect("operation should succeed");
        assert_approx_eq(0.5 * *p, sim.trace_change().expect("state should be valid"));

        // Repeating a projection should yield the same result.
        sim.apply_operation(&mz0, &[1])
            .expect("operation should succeed");
        assert_approx_eq(0.5 * *p, sim.trace_change().expect("state should be valid"));
    }
}

/// Check that projecting the target qubit in a CRX gate on the mz1 direction yields the right probabilities.
pub fn crx_gate_projection_mz1<NS: NoisySimulator>() {
    let (h, mz1) = (h(), mz1());
    let probabilities: Vec<f64> = vec![0.05, 0.1, 0.3, 0.7, 0.8, 0.9, 0.99];

    // A CRX gate (Controlled Rotation around X axis).
    let crx = |t: f64| {
        let c = t.cos();
        let s = t.sin() * Complex::I;
        operation!([1., 0., 0., 0.;
                    0., 1., 0., 0.;
                    0., 0., c,  s;
                    0., 0., s,  c;])
    };

    for p in &probabilities {
        let t = p.sqrt().acos();
        let mut sim = NS::new(2);

        // Apply CRX gate
        sim.apply_operation(&h, &[0])
            .expect("operation should succeed");
        sim.apply_operation(&crx(0.3 * t), &[1, 0])
            .expect("operation should succeed");
        sim.apply_operation(&crx(0.7 * t), &[1, 0])
            .expect("operation should succeed");

        sim.apply_operation(&mz1, &[0])
            .expect("operation should succeed");
        assert_approx_eq(0.5, sim.trace_change().expect("state should be valid"));

        // Project target qubit on mz1 and check the trace
        // (the probability of finding the system in that state).
        sim.apply_operation(&mz1, &[1])
            .expect("operation should succeed");
        assert_approx_eq(
            0.5 * (1. - *p),
            sim.trace_change().expect("state should be valid"),
        );

        // Repeating a projection should yield the same result.
        sim.apply_operation(&mz1, &[1])
            .expect("operation should succeed");
        assert_approx_eq(
            0.5 * (1. - *p),
            sim.trace_change().expect("state should be valid"),
        );
    }
}

/// Check that two consecutive MZ on the same qubit yield the same outcome.
pub fn repeated_mz<NS: NoisySimulator>(seed: u64) {
    let h = h();
    let mz = mz();
    let mut sim = NS::new(1);
    sim.set_rng_seed(seed);

    sim.apply_operation(&h, &[0])
        .expect("operation should succeed");
    let outcome_0 = sim
        .sample_instrument(&mz, &[0])
        .expect("measurement should succeed");
    let outcome_1 = sim
        .sample_instrument(&mz, &[0])
        .expect("measurement should succeed");
    assert_eq!(outcome_0, outcome_1);
}

pub fn alternating_mz_and_mx<NS: NoisySimulator>() {
    let h = h();
    let mz = mz();
    let mx = Instrument::new(vec![
        operation!([0.5, 0.5;
                    0.5, 0.5;]),
        operation!([ 0.5, -0.5;
                    -0.5,  0.5;]),
    ]);

    let mut sim = NS::new(1);
    sim.apply_operation(&h, &[0])
        .expect("operation should succeed");
    let mut prob = 1.0;

    // Alternate MZ and MX 5 times.
    for _ in 0..5 {
        sim.sample_instrument(&mz, &[0])
            .expect("measurement should succeed");
        prob *= 0.5;
        assert_approx_eq(prob, sim.trace_change().expect("state should be valid"));
        sim.sample_instrument(&mx, &[0])
            .expect("measurement should succeed");
        prob *= 0.5;
        assert_approx_eq(prob, sim.trace_change().expect("state should be valid"));
    }
}
