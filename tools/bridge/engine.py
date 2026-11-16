"""The pcb-routing program: its runs, and the questions put to it."""

import json, os, subprocess, tempfile
from dataclasses import dataclass
from pathlib import Path

# The engine refuses a problem of this many entries or more; a board with more is routed in batches.
ENGINE_NETS = 250


class EngineFailed(Exception):
    """The engine gave no solution; the message is what it said."""


def run_engine(problem, engine, threads=0, problem_path=None, solution_path=None):
    """Run the pcb-routing program on a problem and return its solution; its report goes to standard output.

    `engine` is the program (or a command as a list). The problem and the solution are kept under the given
    names, if any.
    """
    with tempfile.TemporaryDirectory() as tmp:
        problem_path = Path(problem_path or Path(tmp) / "problem.json")
        solution_path = Path(solution_path or Path(tmp) / "solution.json")
        problem_path.write_text(json.dumps(problem))
        solution_path.unlink(missing_ok=True)  # an old file must not pass for this run's answer
        command = ([engine] if isinstance(engine, (str, os.PathLike))
                   else list(engine)) + ["route", str(problem_path), "-o", str(solution_path)]
        if threads:
            command += ["--threads", str(threads)]
        try:
            run = subprocess.run(command, capture_output=True, text=True)
        except OSError as error:
            raise EngineFailed(f"cannot run {command[0]}: {error}")
        print(run.stdout, end="")
        if run.returncode == 2 or not solution_path.exists():
            raise EngineFailed(run.stderr.strip() or f"{command[0]} ended with status {run.returncode} and no solution")
        return json.loads(solution_path.read_text())


def ask_engine(question, engine, threads=0):
    """Put a question to the pcb-routing program (its `ask` command: see src/ask.rs) and return the answer.
    `engine` is the program (or a command as a list)."""
    command = ([engine] if isinstance(engine, (str, os.PathLike))
               else list(engine)) + ["ask"] + (["--threads", str(threads)] if threads else [])
    try:
        run = subprocess.run(command, input=json.dumps(question), capture_output=True, text=True)
    except OSError as error:
        raise EngineFailed(f"cannot run {command[0]}: {error}")
    if run.returncode:
        raise EngineFailed(run.stderr.strip() or f"{command[0]} ask ended with status {run.returncode}")
    try:
        return json.loads(run.stdout)
    except ValueError:
        raise EngineFailed(f"{command[0]} ask gave no answer that can be read")


@dataclass
class Engine:
    """The pcb-routing program as `route` uses it."""
    program: object  # the program, or a command as a list
    threads: int = 0  # the threads it may use (0: as many as the machine has)
    problem_path: object = None  # where the problem of the first run is kept, if anywhere
    solution_path: object = None  # and its solution

    def route(self, problem, first=False):
        """Run a problem and return its solution (`first`: the first run of all); EngineFailed if it gives none."""
        return run_engine(problem, self.program, self.threads, self.problem_path if first else None,
                          self.solution_path if first else None)

    def ask(self, question):
        """Put a question (see src/ask.rs) and return the answer; EngineFailed if it gives none."""
        return ask_engine(question, self.program, self.threads)
