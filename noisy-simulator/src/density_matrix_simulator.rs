// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! This module contains two structs: the `DensityMatrixSimulator` and its
//! internal `DensityMatrix` state.

#[cfg(test)]
mod tests;

use crate::{
    handle_error, instrument::Instrument, kernel::apply_kernel, operation::Operation,
    ComplexVector, Error, SquareMatrix, TOLERANCE,
};
use num_complex::Complex;

/// A vectorized density matrix.
#[derive(Debug, Clone)]
pub struct DensityMatrix {
    /// Dimension of the matrix. E.g.: If the matrix is 5 x 5, then dim is 5.
    dim: usize,
    /// Number of qubits in the system.
    number_of_qubits: usize,
    /// Theoretical change in trace due to operations that have been applied so far.
    trace_change: f64,
    /// Vector storing the entries of the density matrix.
    data: ComplexVector,
}

impl DensityMatrix {
    fn new(number_of_qubits: usize) -> Self {
        let dim = 1 << number_of_qubits;
        let mut data = ComplexVector::zeros(dim * dim);
        data[0].re = 1.0;
        Self {
            dim,
            number_of_qubits,
            trace_change: 1.0,
            data,
        }
    }

    /// Builds a `DensityMatrix` from its raw fields. Returns `None` if
    ///  the provided args don't represent a valid `DensityMatrix`.
    ///
    /// This method is to be used from the PyO3 wrapper.
    pub fn try_from(
        dim: usize,
        number_of_qubits: usize,
        trace_change: f64,
        data: ComplexVector,
    ) -> Option<Self> {
        if 1 << number_of_qubits != dim || data.len() != dim * dim {
            None
        } else {
            Some(Self {
                dim,
                number_of_qubits,
                trace_change,
                data,
            })
        }
    }

    /// Returns a reference to the vector containing the density matrix's data.
    pub fn data(&self) -> &ComplexVector {
        &self.data
    }

    /// Returns dimension of the matrix. E.g.: If the matrix is 5 x 5, then dim is 5.
    pub fn dim(&self) -> usize {
        self.dim
    }

    /// Returns the number of qubits in the system.
    pub fn number_of_qubits(&self) -> usize {
        self.number_of_qubits
    }

    /// Returns `true` if the matrix is Hermitian.
    fn is_hermitian(&self) -> bool {
        for row in 0..self.dim {
            for col in 0..self.dim {
                let elt = self.data[self.dim * row + col];
                let mirror_elt = self.data[self.dim * col + row];
                if (elt.re - mirror_elt.re).abs() > TOLERANCE
                    || (elt.im + mirror_elt.im).abs() > TOLERANCE
                {
                    return false;
                }
            }
        }
        true
    }

    /// Returns `true` if the trace of the matrix is 1.
    fn is_normalized(&self) -> bool {
        (self.trace() - 1.0).abs() <= TOLERANCE
    }

    /// Returns the trace of the matrix. The trace is the sum of the diagonal entries of a matrix.
    fn trace(&self) -> f64 {
        let mut trace: Complex<f64> = Complex::ZERO;
        for idx in 0..self.dim {
            trace += self.data[(self.dim + 1) * idx];
        }
        assert!(
            trace.im <= TOLERANCE,
            "state trace is not real, imaginary part is {}",
            trace.im
        );
        trace.re
    }

    /// Return theoretical change in trace due to operations that have been applied so far.
    /// In reality, the density matrix is always renormalized after instruments / operations
    /// have been applied.
    pub fn trace_change(&self) -> f64 {
        self.trace_change
    }

    /// Renormalizes the matrix such that the trace is 1.
    fn renormalize(&mut self) -> Result<(), Error> {
        self.renormalize_with_trace(self.trace())
    }

    /// Renormalizes the matrix such that the trace is 1. Uses a precomputed `trace`.
    fn renormalize_with_trace(&mut self, trace: f64) -> Result<(), Error> {
        if trace < TOLERANCE {
            return Err(Error::ProbabilityZeroEvent);
        }
        self.trace_change *= trace;
        let renormalization_factor = 1.0 / trace;
        for entry in self.data.iter_mut() {
            *entry *= renormalization_factor;
        }
        Ok(())
    }

    /// Applies the operation matrix to the target qubits.
    fn apply_operation_matrix(
        &mut self,
        operation_matrix: &SquareMatrix,
        qubits: &[usize],
    ) -> Result<(), Error> {
        // TODO [Research]: Figure out why they do this qubits_expanded thing.
        let mut qubits_expanded = Vec::with_capacity(2 * qubits.len());
        for id in qubits {
            qubits_expanded.push(*id);
        }
        for id in qubits {
            qubits_expanded.push(*id + self.number_of_qubits());
        }
        apply_kernel(&mut self.data, operation_matrix, &qubits_expanded)
    }
}

/// A quantum circuit simulator using a density matrix.
///
/// All the simulator methods return a `Result<_, Error>`. If the simulator reaches an
/// invalid state due to a numerical error, it will return that last error from there on.
pub struct DensityMatrixSimulator {
    /// A `DensityMatrix` representing the current state of the quantum system.
    state: Result<DensityMatrix, Error>,
    /// Dimension of the density matrix. We need this field to verify the size of the
    /// quantum system in the `set_state` method in the case that `self.state == Err(...)`.
    dim: usize,
}

impl DensityMatrixSimulator {
    /// Creates a new `DensityMatrixSimulator`.
    pub fn new(number_of_qubits: usize) -> Self {
        let density_matrix = DensityMatrix::new(number_of_qubits);
        let dim = density_matrix.dim();
        Self {
            state: Ok(density_matrix),
            dim,
        }
    }

    /// Apply an operation to the given qubit ids.
    pub fn apply_operation(
        &mut self,
        operation: &Operation,
        qubits: &[usize],
    ) -> Result<(), Error> {
        self.state
            .as_mut()?
            .apply_operation_matrix(operation.matrix(), qubits)?;
        if let Err(err) = self.state.as_mut()?.renormalize() {
            handle_error!(self, err);
        }
        Ok(())
    }

    /// Apply non selective evolution to the given qubit ids.
    pub fn apply_instrument(
        &mut self,
        instrument: &Instrument,
        qubits: &[usize],
    ) -> Result<(), Error> {
        self.state
            .as_mut()?
            .apply_operation_matrix(instrument.non_selective_operation_matrix(), qubits)?;
        if let Err(err) = self.state.as_mut()?.renormalize() {
            handle_error!(self, err);
        }
        Ok(())
    }

    /// Performs selective evolution under the given instrument.
    /// Returns the index of the observed outcome.
    ///
    /// Use this method to perform measurements on the quantum system.
    pub fn sample_instrument(
        &mut self,
        instrument: &Instrument,
        qubits: &[usize],
    ) -> Result<usize, Error> {
        self.sample_instrument_with_distribution(instrument, qubits, rand::random())
    }

    /// Performs selective evolution under the given instrument.
    /// Returns the index of the observed outcome.
    pub fn sample_instrument_with_distribution(
        &mut self,
        instrument: &Instrument,
        qubits: &[usize],
        random_sample: f64,
    ) -> Result<usize, Error> {
        let mut tmp_state = self.state.clone()?;
        apply_kernel(
            &mut tmp_state.data,
            instrument.total_effect_transposed(),
            qubits,
        )?;
        let total_effect_trace = tmp_state.trace();
        if total_effect_trace < TOLERANCE {
            let err = Error::ProbabilityZeroEvent;
            handle_error!(self, err);
        }
        let mut last_non_zero_trace_outcome: usize = 0;
        let mut last_non_zero_trace: f64 = 0.0;
        let mut summed_probability: f64 = 0.0;

        for outcome in 0..instrument.num_operations() {
            if summed_probability > random_sample {
                break;
            }
            tmp_state = self.state.clone()?;
            apply_kernel(
                &mut tmp_state.data,
                instrument.operation(outcome).effect_matrix_transpose(),
                qubits,
            )?;
            let outcome_trace = tmp_state.trace();
            summed_probability += outcome_trace / total_effect_trace;
            if outcome_trace >= TOLERANCE {
                last_non_zero_trace_outcome = outcome;
                last_non_zero_trace = outcome_trace;
            }
        }

        if summed_probability + TOLERANCE <= random_sample || last_non_zero_trace < TOLERANCE {
            let err = Error::FailedToSampleInstrumentOutcome;
            handle_error!(self, err);
        }

        if let Err(err) = self.state.as_mut()?.apply_operation_matrix(
            instrument.operation(last_non_zero_trace_outcome).matrix(),
            qubits,
        ) {
            handle_error!(self, err);
        };
        if let Err(err) = self
            .state
            .as_mut()?
            .renormalize_with_trace(last_non_zero_trace)
        {
            handle_error!(self, err);
        };
        Ok(last_non_zero_trace_outcome)
    }

    /// Returns the `DensityMatrix` if the simulator is in a valid state.
    pub fn state(&self) -> Result<&DensityMatrix, &Error> {
        self.state.as_ref()
    }

    /// Set state of the quantum system.
    pub fn set_state(&mut self, new_state: DensityMatrix) -> Result<(), Error> {
        if self.dim != new_state.dim() {
            return Err(Error::InvalidState(format!(
                "the provided state should have the same dimensions as the quantum system's state, {} != {}",
                self.dim,
                new_state.dim(),
            )));
        }
        if !new_state.is_normalized() {
            return Err(Error::InvalidState(format!(
                "`state` is not normalized, trace is {}",
                new_state.trace()
            )));
        }
        if !new_state.is_hermitian() {
            return Err(Error::InvalidState("`state` is not Hermitian".to_string()));
        }
        self.state = Ok(new_state);
        Ok(())
    }

    /// Return theoretical change in trace due to operations that have been applied so far
    /// In reality, the density matrix is always renormalized after instruments/operations
    /// have been applied.
    pub fn trace_change(&self) -> Result<f64, Error> {
        Ok(self.state.as_ref()?.trace_change())
    }

    /// Set the trace of the quantum system.
    pub fn set_trace(&mut self, trace: f64) -> Result<(), Error> {
        if trace < TOLERANCE || (trace - 1.) > TOLERANCE {
            return Err(Error::NotNormalized(trace));
        }
        self.state.as_mut()?.trace_change = trace;
        Ok(())
    }
}
