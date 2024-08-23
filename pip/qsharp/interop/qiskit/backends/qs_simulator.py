# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

from collections import Counter
from concurrent.futures import Executor
import logging
from typing import Any, Dict, List, Optional, Tuple, Union
from uuid import uuid4

from qiskit import QuantumCircuit
from qiskit.providers import Options
from qiskit.transpiler.target import Target
from .... import Result, TargetProfile
from ..execution import DetaultExecutor
from ..jobs import QsSimJob, QsJobSet
from .qsbackend import QsBackend
from .compilation import Compilation

logger = logging.getLogger(__name__)


def _map_qsharp_value_to_bit(v) -> str:
    if isinstance(v, Result):
        if v == Result.One:
            return "1"
        else:
            return "0"
    return str(v)


# Convert Q# output to the result format expected by Qiskit
def _to_qiskit_bitstring(obj):
    if isinstance(obj, tuple):
        return " ".join([_to_qiskit_bitstring(term) for term in obj])
    elif isinstance(obj, list):
        return "".join([_map_qsharp_value_to_bit(bit) for bit in obj])
    else:
        return obj


class QSharpSimulator(QsBackend):
    """
    A virtual backend for running Qiskit circuits using the Q# simulator.
    """

    # This init is included for the docstring
    # pylint: disable=useless-parent-delegation
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
            qiskit_pass_options (Dict): Options for the Qiskit passes.
            transpile_options (Dict): Options for the transpiler.
            qasm_export_options (Dict): Options for the QASM3 exporter.
            **options: Additional options for the execution.
              - name (str): The name of the circuit. This is used as the entry point for the program.
                  The circuit name will be used if not specified.
              - target_profile (TargetProfile): The target profile to use for the compilation.
              - shots (int): The number of shots to run the program for. Defaults to 1024.
              - seed (int): The seed to use for the random number generator. Defaults to None.
              - search_path (str): The path to search for imports. Defaults to '.'.
              - output_fn (Callable[[Output], None]): A callback function to
                  receive the output of the circuit. Defaults to None.
              - executor(ThreadPoolExecutor or other Executor):
                  The executor to be used to submit the job. Defaults to SynchronousExecutor.
        """

        super().__init__(
            target,
            qiskit_pass_options,
            transpile_options,
            qasm_export_options,
            skip_transpilation,
            **fields,
        )

    @classmethod
    def _default_options(cls):
        return Options(
            name="program",
            params=None,
            search_path=".",
            shots=1024,
            seed=None,
            output_fn=None,
            target_profile=TargetProfile.Unrestricted,
            executor=DetaultExecutor(),
        )

    def run(
        self,
        run_input: Union[QuantumCircuit, List[QuantumCircuit]],
        **options,
    ) -> QsSimJob:
        """
        Runs the given QuantumCircuit using the Q# simulator.

        Args:
            run_input (QuantumCircuit): The QuantumCircuit to be executed.
            **options: Additional options for the execution.
              - name (str): The name of the circuit. This is used as the entry point for the program.
                  The circuit name will be used if not specified.
              - params (Optional[str]): The entry expression to use for the program. Defaults to None.
              - target_profile (TargetProfile): The target profile to use for the compilation.
              - shots (int): The number of shots to run the program for. Defaults to 1024.
              - seed (int): The seed to use for the random number generator. Defaults to None.
              - search_path (str): The path to search for imports. Defaults to '.'.
              - output_fn (Callable[[Output], None]): A callback function to
                  receive the output of the circuit.
              - executor(ThreadPoolExecutor or other Executor):
                  The executor to be used to submit the job.
        Returns:
            QSharpJob: The simulation job

        :raises QSharpError: If there is an error evaluating the source code.
        :raises QasmError: If there is an error compiling the source code.
        :raises QiskitError: If there is an error generating or parsing QASM.
        :raises AssertionError: If the run_input is not a QuantumCircuit.
        """

        if not isinstance(run_input, list):
            run_input = [run_input]
        for circuit in run_input:
            assert isinstance(
                circuit, QuantumCircuit
            ), "The run_input must be a QuantumCircuit."

        return self._run(run_input, **options)

    def _execute(self, programs: List[Compilation], **input_params) -> Dict[str, Any]:
        exec_results: List[Tuple[Compilation, Dict[str, Any]]] = [
            (
                program,
                _run_qasm3(program.qasm, vars(self.options).copy(), **input_params),
            )
            for program in programs
        ]
        job_results = []

        shots = input_params.get("shots")
        if shots is None:
            raise ValueError("The number of shots must be specified.")

        for program, exec_result in exec_results:
            results = [_to_qiskit_bitstring(result) for result in exec_result]

            counts = Counter(results)
            counts_dict = dict(counts)
            probabilities = {
                bitstring: (count / shots) for bitstring, count in counts_dict.items()
            }

            job_result = {
                "data": {"counts": counts_dict, "probabilities": probabilities},
                "success": True,
                "header": {
                    "metadata": {"qasm": program.qasm},
                    "name": program.circuit.name,
                    "compilation_time_taken": program.time_taken,
                },
                "shots": shots,
            }
            job_results.append(job_result)

        # All of theses fields are required by the Result object
        result_dict = {
            "results": job_results,
            "qobj_id": str(uuid4()),
            "success": True,
        }

        return result_dict

    def _create_results(self, output: Dict[str, Any]) -> Any:
        from qiskit.result import Result

        result = Result.from_dict(output)
        return result

    def _submit_job(
        self, run_input: List[QuantumCircuit], **options
    ) -> Union[QsSimJob, QsJobSet]:
        job_id = str(uuid4())
        executor: Executor = options.pop("executor", DetaultExecutor())
        if len(run_input) == 1:
            job = QsSimJob(self, job_id, self.run_job, run_input, options, executor)
        else:
            job = QsJobSet(self, job_id, self.run_job, run_input, options, executor)
        job.submit()
        return job


def _run_qasm3(
    qasm: str,
    default_options: Options,
    **options,
) -> Any:
    """
    Runs the supplied OpenQASM 3 program.
    Gates defined by stdgates.inc will be overridden with definitions
    from the Q# compiler.

    Any gates, such as matrix unitaries, that are not able to be
    transpiled will result in an error.

    Parameters:
    source (str): The input OpenQASM 3 string to be processed.
        **options: Additional keyword arguments to pass to the execution.
        - params (Optional[str]): The entry expression to use for the program. Defaults to None.
        - target_profile (TargetProfile): The target profile to use for execution.
        - name (str): The name of the circuit. This is used as the entry point for the program. Defaults to 'program'.
        - search_path (str): The optional search path for resolving qasm3 imports.
        - shots (int): The number of shots to run the program for. Defaults to 1.
        - seed (int): The seed to use for the random number generator.
        - output_fn (Optional[Callable[[Output], None]]): A callback function that will be called with each output. Defaults to None.

    :returns values: A result or runtime errors.

    :raises QSharpError: If there is an error evaluating the source code.
    :raises QasmError: If there is an error compiling the source code.
    :raises QiskitError: If there is an error generating or parsing QASM.
    """

    from ...._native import Output
    from ...._native import run_qasm3
    from ...._fs import read_file, list_directory, resolve
    from ...._http import fetch_github

    def callback(output: Output) -> None:
        print(output)

    output_fn = options.pop("output_fn", callback)

    name = options.pop("name", default_options["name"])
    target_profile = options.pop("target_profile", default_options["target_profile"])
    search_path = options.pop("search_path", default_options["search_path"])
    params = options.pop("params", default_options["params"])
    shots = options.pop("shots", default_options["shots"])
    seed = options.pop("seed", default_options["seed"])

    # when passing the args into the rust layer, any kwargs with None values
    # will cause an error, so we need to filter them out.
    args = {}
    if name is not None:
        args["name"] = name
    if target_profile is not None:
        args["target_profile"] = target_profile
    if search_path is not None:
        args["search_path"] = search_path
    if params is not None:
        args["params"] = params
    if shots is not None:
        args["shots"] = shots
    if seed is not None:
        args["seed"] = seed

    return run_qasm3(
        qasm,
        output_fn,
        read_file,
        list_directory,
        resolve,
        fetch_github,
        **args,
    )
