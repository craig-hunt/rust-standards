#!/usr/bin/env python3
"""Reports what the mutation run actually measured, and fails below the gate.

cargo-mutants prints a summary, and the summary is not the thing to trust. A
mutant that timed out was never tested: the run gave up on it. Counting one as
caught is how a gate reports a healthy number for code nothing examined, which is
exactly what happened on the Java sibling before anyone read the per-status
breakdown.

So this reads the outcomes file, reports killed separately from timed out, and
treats any timeout as an unmeasured region rather than as a pass.
"""

import json
import pathlib
import sys

OUTCOMES = pathlib.Path("mutants.out/outcomes.json")

CAUGHT = "CaughtMutant"
MISSED = "MissedMutant"
TIMEOUT = "Timeout"
UNVIABLE = "Unviable"


def main() -> int:
    threshold = int(sys.argv[1])

    if not OUTCOMES.is_file():
        print(f"no mutation outcomes at {OUTCOMES}", file=sys.stderr)
        return 1

    outcomes = json.loads(OUTCOMES.read_text(encoding="utf-8"))
    summary = {CAUGHT: 0, MISSED: 0, TIMEOUT: 0, UNVIABLE: 0}
    missed = []

    for outcome in outcomes.get("outcomes", []):
        status = outcome.get("summary", "")
        summary[status] = summary.get(status, 0) + 1
        if status == MISSED:
            scenario = outcome.get("scenario", {})
            mutant = scenario.get("Mutant", {})
            missed.append(
                f"  {mutant.get('file', '?')}:{mutant.get('line', '?')} "
                f"{mutant.get('replacement', '?')}"
            )

    # Unviable mutants did not compile, so no test could have caught them. They
    # are not a gap and they are not a kill; they are excluded from both sides.
    tested = summary[CAUGHT] + summary[MISSED] + summary[TIMEOUT]
    if tested == 0:
        print("no mutants were tested", file=sys.stderr)
        return 1

    honest = summary[CAUGHT] * 100 // tested

    print(f"  tested      {tested}")
    print(f"  caught      {summary[CAUGHT]}")
    print(f"  missed      {summary[MISSED]}")
    print(f"  timed out   {summary[TIMEOUT]}")
    print(f"  unviable    {summary[UNVIABLE]} (did not compile, excluded)")
    print(f"  honest kill {honest}% against a gate of {threshold}%")

    if missed:
        print("\nmutants nothing caught:")
        print("\n".join(sorted(missed)))

    if summary[TIMEOUT] > 0:
        print(
            f"\n{summary[TIMEOUT]} mutant(s) timed out, which means they were never "
            "tested rather than that they were caught.",
            file=sys.stderr,
        )
        return 1

    if honest < threshold:
        print(f"\n{honest}% is below the gate of {threshold}%", file=sys.stderr)
        return 1

    return 0


if __name__ == "__main__":
    sys.exit(main())
