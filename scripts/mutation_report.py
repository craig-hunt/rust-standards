#!/usr/bin/env python3
"""Reports what the mutation run actually measured, and fails below the gate.

cargo-mutants prints a summary, and the summary is not the thing to trust. A
mutant that timed out was never tested: the run gave up on it. Counting one as
caught is how a gate reports a healthy number for code nothing examined, which is
exactly what happened on the Java sibling before anyone read the per-status
breakdown.

So this reads the outcomes file, reports killed separately from timed out, and
treats any timeout as an unmeasured region rather than as a pass.

It also refuses to score a run it cannot prove completed. The mutation step used
to end in `|| true`, which swallowed every non-zero exit alike: a crash, an
interrupted run and a baseline that would not build all looked the same as a
survivor, and a report left over from an earlier run would then have satisfied
the gate on its own. Three things have to hold before a number means anything:
the run wrote an end time, every mutant it found is accounted for in the tally,
and a non-zero exit is explained by something in the report.
"""

import json
import pathlib
import sys

OUTCOMES = pathlib.Path("mutants.out/outcomes.json")

BASELINE = "Baseline"
SUCCESS = "Success"
CAUGHT = "CaughtMutant"
MISSED = "MissedMutant"
TIMEOUT = "Timeout"
UNVIABLE = "Unviable"

PERCENT = 100
CLEAN_EXIT = 0
FAILED = 1


def read_outcomes():
    """The report this run wrote, or a reason it cannot be scored."""
    if not OUTCOMES.is_file():
        return None, f"no mutation outcomes at {OUTCOMES}"

    try:
        report = json.loads(OUTCOMES.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as unreadable:
        return None, f"{OUTCOMES} did not read as JSON: {unreadable}"

    # Written when the run finishes rather than as it goes, so its absence means
    # the run was killed partway and the counts below describe a fraction of the
    # work nobody can name.
    if not report.get("end_time"):
        return None, f"{OUTCOMES} has no end time, so that run never finished"

    return report, None


def baseline_passed(report) -> bool:
    """Whether the unmutated tree built and tested cleanly.

    A baseline that fails means every mutant after it was scored against a tree
    that was already broken, so the kill rate describes nothing.
    """
    for outcome in report.get("outcomes", []):
        if outcome.get("scenario") == BASELINE:
            return outcome.get("summary") == SUCCESS
    return False


def main() -> int:
    threshold = int(sys.argv[1])
    exit_status = int(sys.argv[2])

    report, unscorable = read_outcomes()
    if unscorable:
        print(unscorable, file=sys.stderr)
        return FAILED

    if not baseline_passed(report):
        print(
            "the unmutated tree did not pass its own tests, so no mutant score "
            "from this run means anything",
            file=sys.stderr,
        )
        return FAILED

    tally = {CAUGHT: 0, MISSED: 0, TIMEOUT: 0, UNVIABLE: 0}
    missed = []

    for outcome in report.get("outcomes", []):
        if outcome.get("scenario") == BASELINE:
            continue
        status = outcome.get("summary", "")
        tally[status] = tally.get(status, 0) + 1
        if status == MISSED:
            scenario = outcome.get("scenario", {})
            mutant = scenario.get("Mutant", {})
            missed.append(
                f"  {mutant.get('file', '?')}:{mutant.get('line', '?')} "
                f"{mutant.get('replacement', '?')}"
            )

    # Unviable mutants did not compile, so no test could have caught them. They
    # are not a gap and they are not a kill; they are excluded from both sides.
    tested = tally[CAUGHT] + tally[MISSED] + tally[TIMEOUT]
    accounted = tested + tally[UNVIABLE]
    found = report.get("total_mutants", 0)

    if tested == 0:
        print("no mutants were tested", file=sys.stderr)
        return FAILED

    honest = tally[CAUGHT] * PERCENT // tested

    print(f"  tested      {tested}")
    print(f"  caught      {tally[CAUGHT]}")
    print(f"  missed      {tally[MISSED]}")
    print(f"  timed out   {tally[TIMEOUT]}")
    print(f"  unviable    {tally[UNVIABLE]} (did not compile, excluded)")
    print(f"  honest kill {honest}% against a gate of {threshold}%")

    if missed:
        print("\nmutants nothing caught:")
        print("\n".join(sorted(missed)))

    if accounted != found:
        print(
            f"\nthe run found {found} mutants and reported on {accounted}: "
            "it stopped before it measured everything it set out to",
            file=sys.stderr,
        )
        return FAILED

    if tally[TIMEOUT] > 0:
        print(
            f"\n{tally[TIMEOUT]} mutant(s) timed out, which means they were never "
            "tested rather than that they were caught.",
            file=sys.stderr,
        )
        return FAILED

    # A survivor or a timeout explains a non-zero exit. Anything else means the
    # tool stopped for a reason this report cannot see, and a gate that passes on
    # a reason nobody read is not a gate.
    unexplained = exit_status != CLEAN_EXIT and not missed and tally[TIMEOUT] == 0
    if unexplained:
        print(
            f"\ncargo-mutants exited {exit_status} with nothing missed and nothing "
            "timed out, so it stopped for a reason this report cannot account for",
            file=sys.stderr,
        )
        return FAILED

    if honest < threshold:
        print(f"\n{honest}% is below the gate of {threshold}%", file=sys.stderr)
        return FAILED

    return CLEAN_EXIT


if __name__ == "__main__":
    sys.exit(main())
