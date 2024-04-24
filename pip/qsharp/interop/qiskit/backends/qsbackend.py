# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

from abc import ABC, abstractmethod
import datetime
import logging
import time
from typing import Dict, Any, List, Optional, Tuple, Union
from warnings import warn

from qiskit import transpile
from qiskit.qobj import QobjHeader
from qiskit.circuit import (
    QuantumCircuit,
)

from qiskit.qasm3.exporter import Exporter, GlobalNamespace
from qiskit.providers import BackendV2, Options
from qiskit.result import Result
from qiskit.transpiler import PassManager
from qiskit.transpiler.passes import RemoveBarriers, RemoveResetInZeroState
from qiskit.transpiler.target import Target

from .qirtarget import QirTarget
from ..jobs import QsJob
from ..passes import RemoveDelays


logger = logging.getLogger(__name__)


def filter_kwargs(func, **kwargs) -> Dict[str, Any]:
    import inspect

    sig = inspect.signature(func)
    supported_args = set(sig.parameters.keys())
    extracted_kwargs = {
        k: kwargs.get(k) for k in list(kwargs.keys()) if k in supported_args
    }
    return extracted_kwargs


def get_transpile_options(**kwargs) -> Dict[str, Any]:
    args = filter_kwargs(transpile, **kwargs)
    if "optimization_level" not in args:
        args["optimization_level"] = 0
    return args


def get_exporter_options(**kwargs) -> Dict[str, Any]:
    return filter_kwargs(Exporter.__init__, **kwargs)


class QsBackend(BackendV2, ABC):
    """
    A virtual backend for transpiling to a Q# ecosystem compatible format.
    """

    def __init__(
        self,
        target: Optional[Target] = None,
        qiskit_pass_options: Optional[Dict[str, Any]] = None,
        transpile_options: Optional[Dict[str, Any]] = None,
        qasm_export_options: Optional[Dict[str, Any]] = None,
        skip_transpilation: bool = False,
        **fields,
    ):
        """
        Parameters:
            target (Target): The target to use for the backend.
            **options: Additional keyword arguments to pass to the
                execution used by subclasses.
        """
        super().__init__(
            name="QSharpSimulator",
            description="A virtual BackendV2 for transpiling to a Q# compatible format.",
            backend_version="0.0.1",
        )

        if fields is not None:
            # we need to rename the seed_simulator to seed. This
            # is a convenience for aer users.
            # if the user passes in seed_simulator, we will rename it to seed
            # but only if the seed field is defined in the backend options.
            if "seed_simulator" in fields and "seed" in self._options.data:
                warn("seed_simulator passed, but field is called seed.")
                fields["seed"] = fields.pop("seed_simulator")

            # updates the options with the fields passed in, if the backend
            # doesn't have the field, it will raise an error.
            self.set_options(**fields)

        self._qiskit_pass_options = Options(
            supports_barrier=False,
            supports_delay=False,
            remove_reset_in_zero_state=True,
        )
        self._skip_transpilation = skip_transpilation

        # we need to set the target after the options are set
        # so that the target_profile can be used to determine
        # which gates/instructions are available
        if target is not None:
            # update the properties so that we are internally consistent
            self._qiskit_pass_options.update_options(
                **{
                    "supports_barrier": target.instruction_supported("barrier"),
                    "supports_delay": target.instruction_supported("delay"),
                    "remove_reset_in_zero_state": True,
                }
            )

            self._target = target
        else:
            self._target = self._create_target()

        self._transpile_options = {}

        basis_gates = None
        if qasm_export_options is not None and "basis_gates" in qasm_export_options:
            basis_gates = qasm_export_options.pop("basis_gates")
        else:
            # here we get the gates that are in the target but not in qasm's
            # stdgates so that we can build the basis gates list for the exporter.
            # A user can override this list by passing in a basis_gates list
            non_gate_instructions = [
                "control_flow",
                "if_else",
                "switch_case",
                "while_loop",
                "break",
                "continue",
                "store",
                "for_loop",
                "measure",
            ]

            target_gates = set(self.target.operation_names)
            target_gates -= set(non_gate_instructions)
            qiskit_gates = set(GlobalNamespace.qiskit_gates.keys())
            basis_gates = list(target_gates - qiskit_gates)

        # selt the default options for the exporter
        self._qasm_export_options = {
            "includes": ("stdgates.inc",),
            "alias_classical_registers": False,
            "allow_aliasing": False,
            "disable_constants": True,
            "basis_gates": basis_gates,
        }

        if qiskit_pass_options is not None:
            self._qiskit_pass_options.update_options(**qiskit_pass_options)
        if transpile_options is not None:
            self._transpile_options.update(**transpile_options)
        if qasm_export_options is not None:
            self._qasm_export_options.update(**qasm_export_options)

    def _create_target(self) -> Target:
        supports_barrier = self._qiskit_pass_options["supports_barrier"]
        supports_delay = self._qiskit_pass_options["supports_delay"]
        return QirTarget(
            target_profile=self._options["target_profile"],
            supports_barrier=supports_barrier,
            supports_delay=supports_delay,
        )

    @property
    def target(self) -> Target:
        """Returns the target of the Backend object."""
        return self._target

    @property
    def max_circuits(self):
        """
        Returns the maximum number of circuits that can be executed simultaneously.
        """
        return None

    @abstractmethod
    def _execute(
        self, programs: List[Tuple[QuantumCircuit, str]], **input_params
    ) -> Dict[str, Any]:
        """Execute circuits on the backend.

        Parameters:
            circuits (List of str): simulator qasm input.
            input_params (Dict): configuration for simulation/compilation.

        Returns:
            dict: return a dictionary of results.
        """

    @abstractmethod
    def run(
        self,
        run_input: Union[QuantumCircuit, List[QuantumCircuit]],
        **options,
    ) -> QsJob:
        pass

    def _run(
        self,
        run_input: List[QuantumCircuit],
        **options,
    ) -> QsJob:
        if "name" not in options and len(run_input) == 1:
            options["name"] = run_input[0].name

        # Get out default options
        # Look at all of the kwargs and see if they match any of the options
        # If they do, set the option to the value of the kwarg as an override
        # We only to remove the options that are in the backend options for
        # the run so that other options can be passed to other calls.
        input_params: Dict[str, Any] = vars(self.options).copy()
        input_params.update(options)

        return self._submit_job(run_input, **input_params)

    def run_job(
        self, run_input: List[QuantumCircuit], job_id: str, **options
    ) -> Result:
        start = time.time()

        compilations = self._compile(run_input, **options)

        output = self._execute(compilations, **options)

        if not isinstance(output, dict):
            logger.error("%s: run failed.", self.name)
            if output:
                logger.error("Output: %s", output)
            from .... import QSharpError

            raise QSharpError("Run terminated without valid output.")

        output["job_id"] = job_id
        output["date"] = str(datetime.datetime.now().isoformat())
        output["status"] = "COMPLETED"
        output["backend_name"] = self.name
        output["backend_version"] = self.backend_version
        output["time_taken"] = str(time.time() - start)
        output["config"] = {
            "qasm_export_options": str(self._build_qasm_export_options(**options)),
            "qiskit_pass_options": str(self._build_qiskit_pass_options(**options)),
            "transpile_options": str(self._build_transpile_options(**options)),
        }
        output["header"] = QobjHeader().to_dict()
        return self._create_results(output)

    @abstractmethod
    def _submit_job(self, run_input: List[QuantumCircuit], **input_params) -> QsJob:
        pass

    def _compile(
        self, run_input: List[QuantumCircuit], **options
    ) -> List[Tuple[QuantumCircuit, str]]:
        # for each run input, convert to qasm3
        compilations = []
        for circuit in run_input:
            args = options.copy()
            assert isinstance(
                circuit, QuantumCircuit
            ), "Input must be a QuantumCircuit."
            compilations.append((circuit, self.qasm3(circuit, **args)))
        return compilations

    @abstractmethod
    def _create_results(self, output: Dict[str, Any]) -> Any:
        pass

    def _transpile(self, circuit: QuantumCircuit, **options) -> QuantumCircuit:
        if self._skip_transpilation:
            return circuit

        circuit = self.run_qiskit_passes(circuit, options)

        orig = self.target.num_qubits
        try:
            self.target.num_qubits = circuit.num_qubits
            transpile_options = get_transpile_options(**options)
            backend = transpile_options.pop("backend", self)
            target = transpile_options.pop("target", self.target)
            transpiled_circuit = transpile(
                circuit, backend=backend, target=target, **transpile_options
            )
            return transpiled_circuit
        finally:
            self.target.num_qubits = orig

    def run_qiskit_passes(self, circuit, options):
        pass_options = self._build_qiskit_pass_options(**options)

        pass_manager = PassManager()
        if not pass_options["supports_barrier"]:
            pass_manager.append(RemoveBarriers())
        if not pass_options["supports_delay"]:
            pass_manager.append(RemoveDelays())
        if pass_options["remove_reset_in_zero_state"]:
            # when doing state initialization, qiskit will reset all qubits to 0
            # As our semantics are different, we can remove these resets
            # as it will double the number of qubits if we have to reset them
            # before using them when using the base profile.
            pass_manager.append(RemoveResetInZeroState())

        circuit = pass_manager.run(circuit)
        return circuit

    def _build_qiskit_pass_options(self, **kwargs) -> Dict[str, Any]:
        params: Dict[str, Any] = vars(self._qiskit_pass_options).copy()
        for opt in params.copy():
            if opt in kwargs:
                params[opt] = kwargs.pop(opt)
        if "supports_barrier" not in params:
            params["supports_barrier"] = False
        if "supports_delay" not in params:
            params["supports_delay"] = False
        if "remove_reset_in_zero_state" not in params:
            params["remove_reset_in_zero_state"] = True

        return params

    def _build_transpile_options(self, **kwargs) -> Dict[str, Any]:
        # create the default options from the backend
        args = self._transpile_options.copy()
        # gather any remaining options that are not in the default list
        transpile_args = get_transpile_options(**kwargs)
        args.update(transpile_args)
        return args

    def _build_qasm_export_options(self, **kwargs) -> Dict[str, Any]:
        # Disable aliasing until we decide want to support it
        # The exporter defaults to only having the U gate.
        # When it sees the stdgates.inc in the default includes list, it adds
        # bodyless symbols for that fixed gate set.
        # We set the basis gates for any gates that we want that wouldn't
        # be defined when stdgates.inc is included.

        # any gates that are not in the stdgates.inc file need to be defined
        # in the basis gates list passed to the exporter. The exporter doesn't
        # know about the gates defined in the backend's target.
        # Anything in the basis_gates gets added to the qasm builder's global
        # namespace as an opaque gate. All parameter information comes from the
        # gate object itself in the circuit.

        # create the default options from the backend
        args = self._qasm_export_options.copy()
        # gather any remaining options that are not in the default list
        exporter_args = get_exporter_options(**kwargs)
        args.update(exporter_args)
        return args

    def transpile(self, circuit: QuantumCircuit, **options) -> QuantumCircuit:
        transpiled_circuit = self._transpile(circuit, **options)
        return transpiled_circuit

    def qasm3(self, circuit: QuantumCircuit, **options) -> str:
        """Converts a Qiskit QuantumCircuit to QASM 3 for the current backend.

        Returns:
            str: The converted QASM3 code as a string. Any supplied includes
            are emitted as include statements at the top of the program.
        """

        try:
            export_options = self._build_qasm_export_options(**options)
            transpiled_circuit = self.transpile(circuit, **options)
            exporter = Exporter(**export_options)
            qasm3_source = exporter.dumps(transpiled_circuit)
            return qasm3_source
        except Exception as ex:
            from .. import QiskitError

            raise QiskitError("Failed to export QASM3 source.") from ex

    def qsharp(self, circuit: QuantumCircuit, **kwargs) -> str:
        """
        Converts a Qiskit QuantumCircuit to Q# for the current backend.

        The generated Q# code will not be idiomatic Q# code, but will be
        a direct translation of the Qiskit circuit.

        Qiskit and Q# have different qubit and initialization semantics.
        - Q# assumes that qubits are in the |0⟩ state when they are allocated.
        - Qiskit does not make this assumption and qubits are in an undefined
            state.
        - Q# requires that qubits are reset to the |0⟩ at the end of excution.
        - Qiskit does not require this.
        - Q# does no allow for variables to be uninitialized. All
            initialization is explicit.
        - Qiskit allows for implicit initialization.
        - Q# does not allow for implicit casting or promotion of types. All
            conversions must be explicit.
        - Qiskit allows for implicit casting and promotion of types following
            C99 and custom rules.
        - Q# does not have unsigned integers or an angle type.
            All integers are signed.

        Returns:
            str: The converted QASM3 code as a string. Any supplied includes
            are emitted as include statements at the top of the program.
        """

        qasm3_source = self.qasm3(circuit, **kwargs)

        args = {
            "name": kwargs.get("name", circuit.name),
            "search_path": kwargs.get("search_path", "."),
        }
        qsharp_source = self._qsharp(qasm3_source, **args)
        return qsharp_source

    def qir(
        self,
        circuit: QuantumCircuit,
        **kwargs,
    ) -> str:
        """
        Converts a Qiskit QuantumCircuit to QIR (Quantum Intermediate Representation).

        Args:
            circuit ('QuantumCircuit'): The input Qiskit QuantumCircuit object.
            **kwargs: Additional options for the execution.
              - params (str, optional): The entry expression for the QIR conversion. Defaults to None.
              - target_profile (TargetProfile, optional): The target profile for the backend. Defaults to backend config value.
              - search_path (str, optional): The search path for the backend. Defaults to '.'.

        Returns:
            str: The converted QIR code as a string.
        """
        name = kwargs.pop("name", circuit.name)
        target_profile = kwargs.pop("target_profile", self.options.target_profile)

        qasm3_source = self.qasm3(circuit, **kwargs)

        qir_args = {
            "name": name,
            "target_profile": target_profile,
            "search_path": kwargs.pop("search_path", "."),
        }
        params = kwargs.pop("params", None)
        if params is not None:
            qir_args["params"] = params

        return self._qir(qasm3_source, **qir_args)

    def _qir(
        self,
        source: str,
        **kwargs,
    ) -> str:
        from ...._native import compile_qasm3_to_qir
        from ...._fs import read_file, list_directory, resolve
        from ...._http import fetch_github

        return compile_qasm3_to_qir(
            source,
            read_file,
            list_directory,
            resolve,
            fetch_github,
            **kwargs,
        )

    def _qsharp(
        self,
        source: str,
        **kwargs,
    ) -> str:
        from ...._native import compile_qasm3_to_qsharp
        from ...._fs import read_file, list_directory, resolve
        from ...._http import fetch_github

        return compile_qasm3_to_qsharp(
            source,
            read_file,
            list_directory,
            resolve,
            fetch_github,
            **kwargs,
        )
