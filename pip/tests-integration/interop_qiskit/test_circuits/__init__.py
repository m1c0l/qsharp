# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

from .random import *
from .test_circuits import *

core_tests = [] + random_fixtures
core_tests_small = [] + random_fixtures_small


def generate_repro_information(
    circuit: "QuantumCircuit", target_profile: "TargetProfile", **options
):
    name = circuit.name
    profile_name = str(target_profile)
    message = f"Error with Qiskit circuit '{name}'."
    message += "\n"
    message += f"Profile: {profile_name}"
    message += "\n"
    from qsharp.interop import QSharpSimulator

    try:
        backend = QSharpSimulator(target_profile=target_profile)
        qasm3_source = backend.qasm3(circuit, **options)
    except Exception as ex:
        # if the conversion fails, print the circuit as a string
        # as a fallback since we won't have the qasm3 source
        message = "Failed converting QuantumCircuit to QASM3:\n"
        message += str(ex)
        message += "\n"
        message += "QuantumCircuit rendered:"
        message += "\n"
        circuit_str = str(circuit.draw(output="text"))
        message += circuit_str
        return message

    message += "QASM3 source:"
    message += "\n"
    message += str(qasm3_source)
    return message
