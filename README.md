# Rust Standards

A working service, not a style guide. Every standard below is enforced by
something that fails the build, and the code is here so you can read the
standard rather than take its word for it.

Rust 1.99, edition 2024, PostgreSQL. No application framework.

## Running it

```
docker compose up --build
```

The migrator applies the schema and exits, then the API starts on port 8080.

```
export API_TOKEN=pick-something
curl localhost:8080/health
curl -H "Authorization: Bearer ${API_TOKEN}" localhost:8080/api/inventory
```

`compose.yaml` reads `API_TOKEN` from the same environment, so exporting it
before `docker compose up` gives the service and the client the same value. The
service refuses to start without one.

Every gate, in one command:

```
./scripts/verify.sh
```

That needs the toolchain in `rust-toolchain.toml`, plus `cargo-mutants`,
`cargo-deny` and `gitleaks`. On a machine without them:

```
./scripts/verify-in-docker.sh
```

which builds `scripts/toolchain.Dockerfile`, pinning every tool by digest, and
runs the same `verify.sh` inside it.

## The standards

**1. Dependencies point inward, and the compiler says so.** `domain` knows
nothing of `application`, `application` owns the ports `infrastructure`
implements, and `web` wires them. This is where Rust beats its siblings
outright: Cargo refuses a dependency cycle and a crate cannot use a crate it
does not list, so the boundary is a compile error rather than a test assertion.
The Go sibling asserts it with a hand-written check and the Java one with
ArchUnit; both assert after the fact what Cargo makes unrepresentable.

**2. What Cargo cannot say, a test says.** Nothing stops somebody adding `tokio`
to the domain, and the compiler would be content. `conventions` asserts which
crates each layer may list: the domain may name `thiserror` and `uuid` and
nothing else, and no database client, pool, runtime or HTTP server appears
outside the two outer crates. Proved by adding `tokio` to the domain and
watching it fail.

**3. A value with rules has a type, and there is one way in.** `TaskId` is not
an `i64`, so passing a signup identifier where a task identifier belongs does
not compile. Every field is private and every constructor validates, so a
`TaskTitle` that is empty, untrimmed or over-long has no representation.

**4. A struct of validated types needs no guard clause.** A `Signup` is built
from a `FullName`, an `EmailAddress`, a `Plan`, a `Seats` and a `Notes`. It
cannot be assembled out of a blank email or a plan nobody offers, so no caller
has to remember to validate first. `validate` still earns its place, because a
constructor can only report the first problem it finds and a form needs all of
them.

**5. Every literal with a meaning has a name.** Enforced by a test that parses
every source file with `syn`, main and test alike, and reports any literal
outside a `const` or an attribute. Zero, one and the empty string are excused,
because naming them produces `ZERO` and `EMPTY` and the noise teaches people to
ignore the rule.

The mechanism was chosen rather than defaulted to. Clippy has no lint for this
and nothing in `restriction` distinguishes a literal in a constant from one in a
function body. A `dylint` lint runs inside a compiler driver pinned to one
toolchain, so the repository would carry a second toolchain pin that has to move
in step with `rust-toolchain.toml`. A test names the file and the line, runs
under the same command as everything else, and can be read by somebody who has
never written a lint. The Go sibling reaches the same conclusion.

It earned its place immediately: 54 violations on its first run, including in
its own file, and a gap in itself. It skipped free-standing `const` items and not
*associated* ones, so it was reporting the array length in every
`const ALL: [Self; 3]`. Both failure modes are now covered by tests that build
an offender by hand rather than trusting the sweep.

**6. Failures return, and the compiler matches them.** Go returns errors and
matches them by identity with `errors.Is`. Rust matches them by pattern, which is
the same guarantee enforced by the compiler rather than by a convention about
sentinel values. `DomainError` has two variants because a status depends on
exactly two questions, and a third would stop the build at the mapper until
somebody decided what status it deserves.

**7. Nothing panics in a service.** `unwrap_used`, `expect_used`, `panic`, `todo`
and `unimplemented` are denied workspace-wide. A panic is a request that dies
without an answer and a process that may take its neighbours with it. Test
modules lift those three with a comment saying why, scoped so production code
cannot reach for them: a test asserts by panicking, so a test that cannot panic
cannot fail.

**8. No `unsafe`, and the exemption path goes through review.**
`unsafe_code = "forbid"` rather than `deny`, deliberately: `forbid` cannot be
lifted by an inner `allow`, so an exemption has to be argued for in the
workspace manifest where a reviewer sees it. Go has nothing to say here and Rust
must.

**9. No cast that can wrap.** `cast_possible_truncation` and `cast_sign_loss` are
denied. An integer cast that silently truncates is a defect this codebase should
not be able to write, and the lint caught two in a test fake on the first run.

**10. One place turns a failure into a response.** No handler carries a
catch-all. A domain type that rejects its input returns an error, and the one
mapper in `problem` turns that into a status and a body. A per-handler catch is a
per-handler chance to answer 200 for a failure, or to put internals in a body.

**11. Every failure answers in one shape.** RFC 9457 problem details, with the
stable code and the per-field problems as extension members, on
`application/problem+json`. The Go sibling answers with a bare error object; this
is the documented divergence. `fields` is omitted when empty rather than written
as `{}`, and both ways of building a `Problem` normalize, so the contract holds
however it was built.

**12. A 500 says nothing about the internals.** The driver's message names the
constraint or the relation, and that reaches the log. The body carries a generic
detail, because a message naming a table tells an attacker more than it tells the
caller.

**13. Validation reports every problem at once.** A form marks all its bad fields
in one round trip rather than one field per submission. Each check is the value
type's own constructor rather than a copy of its rule, so a sentence shown to a
user is written in exactly one place.

**14. Ports belong to the caller.** `TaskStore` lives in `application` beside the
service that calls it, not beside the PostgreSQL type that implements it, so
`infrastructure` depends on `application` and never the reverse. A test asserts
that nothing in the workspace depends on `conventions`: the rules read the code,
and the code does not read the rules.

**15. Migration is an admin process.** A separate binary applies the schema and
exits. The API never migrates at startup, so two replicas rolling out together
cannot run the same DDL at the same moment, and PostgreSQL answers concurrent DDL
with a lock wait or a deadlock rather than a tidy no-op.

**16. State and the event announcing it commit together.** Recording a signup
writes the signup row and the outbox row in one transaction, and so does
completing a task. Either both land or neither does, so no consumer hears about a
change that failed to store and no stored change goes unannounced.

Completion announces on the transition only. The statement locks the row and
returns what the flag was beforehand, so setting a completed task completed again
adds nothing to the backlog, and reopening one announces nothing because no event
describes that.

**17. Consumers absorb repeats.** The relay publishes after the commit, which
makes delivery at-least-once: it can deliver a message and fail before recording
that it did. The claim runs `FOR UPDATE SKIP LOCKED`, so a second replica takes a
different batch rather than manufacturing a duplicate on every pass. A consumer
that fails stops the pass, leaving the message pending, because carrying on would
acknowledge a message one consumer never handled.

**18. A message the deployment cannot resolve stays pending.** Marking it would
acknowledge something no consumer saw, which is the one outcome an outbox exists
to prevent. The wire name is a literal, not a type name read at runtime, so
renaming a variant cannot strand a backlog, and a stored name is mapped through a
table rather than resolved, so nothing that can write a row can choose what gets
constructed.

**19. No lock wait is unbounded.** Every pooled connection runs
`SET lock_timeout` as it is handed out. PostgreSQL waits forever by default,
which turns contention into a request that never answers and never errors: no log
line, no metric, nothing for an operator to act on. A test asserts the setting
applies, because a line that applies it is otherwise indistinguishable from a
line that does not.

**20. The clock and the environment arrive as dependencies.** An event carries
the moment it happened, and reading that from the platform directly leaves no way
to assert the value a test expects. Settings are read through a lookup function
rather than from the process environment, because a test cannot set process
variables in Rust without racing every other test in the binary.

**21. Settings are checked once, at startup.** A missing database URL discovered
on the first request is an outage; discovered at startup it is a deployment that
never went live. A blank value counts as absent, because an empty string in a
deployment file is somebody's placeholder and accepting one as a token means
starting a service whose check compares against nothing.

**22. Authorization is asked of the surface, not of the handler.** The bearer
check runs on the surface a path belongs to, before any handler, so a route added
later cannot be published without a token. The comparison does not stop at the
first differing byte: a plain equality check leaks, through timing, how much of a
guess was right.

**23. Every request carries an identifier.** `X-Request-ID` is accepted when it
is short and made of safe characters, and replaced when it is not, because an
unfiltered value travels into both the log and a response header.

**24. Liveness and readiness answer different questions.** Liveness answers as
long as the process can serve a request. Readiness answers for the dependencies.
Collapsing them turns a database outage into a restart loop. Readiness runs under
a deadline, because a probe that waits as long as the database takes is a hung
probe.

**25. Blocking work does not run on a runtime worker.** The ports block, so the
outbox relay runs on a thread of its own. A blocking call on a runtime worker
starves every other request sharing that worker, which is the one mistake this
arrangement has to avoid.

**26. Tests read as behavior statements, and mutation analysis checks that they
mean it.** `cargo-mutants` over `domain` and `application`, gated at 70%.

The number is read from the outcomes file, not from the summary line. A mutant
that timed out was never tested: the run gave up on it, and counting one as
caught is how a gate reports a healthy figure for code nothing examined. Any
timeout fails the gate, and the report prints caught, missed and timed out
separately. That rule exists because the Java sibling reported 82% while 44% of
its mutants had genuinely been killed, and the build passed.

**27. A known vulnerability fails the build.** `cargo-deny`, which answers the
advisory question, the licence question and the source question from one reading
of the lockfile. Permissive licences are listed rather than discovered, crates
come from the registry rather than from a git revision somebody can move, and a
wildcard version is refused.

**28. A secret in history is found before it is pushed.** `gitleaks` over the
tree. The Go sibling runs this and neither the C# nor the Java one does, which is
the gap this closes.

**29. A tool that is absent fails rather than being skipped.** Every gate in
`verify.sh` checks for its tool and exits when it is missing. A check that
quietly did not run is indistinguishable from one that found nothing.

**30. The toolchain is pinned by digest.** `rust-toolchain.toml` names an exact
compiler, and `scripts/toolchain.Dockerfile` addresses every image by digest with
the readable tag in a comment. A file claiming to pin a toolchain while naming a
floating tag is worse than one that makes no claim.

**31. Every version lives in one place.** The workspace manifest, the way
`Directory.Packages.props` works in the C# sibling and the parent POM in the
Java one. A crate that pinned its own version would let two crates disagree about
one dependency with nothing to say so.

**32. Modules are named for what they do.** A test rejects `util`, `helper`,
`common`, `misc`, `impl` and the rest, nested or not, file names included. A
module nobody can describe is a module everything belongs in.

**33. Comments explain why, never what.** Names carry the what. The comments here
record the reasoning a reader would otherwise have to reconstruct, and several of
them exist because the obvious alternative is wrong in a way that is not obvious.

## Layout

```
domain/          the rules, depending on two value-type crates
application/     what the service does, and the ports it needs
infrastructure/  the adapters, and every dependency that leaves the process
web/             routing, JSON, problem details, and the wiring
conventions/     the standards above, as tests that fail
```

## API

| Method | Path                | Answers                                         |
| ------ | ------------------- | ----------------------------------------------- |
| GET    | `/health`           | liveness, no token                              |
| GET    | `/health/ready`     | readiness, no token                             |
| GET    | `/api/tasks`        | the task list, `?filter=all\|active\|completed`  |
| POST   | `/api/tasks`        | creates a task                                  |
| PATCH  | `/api/tasks/{id}`   | sets completion                                 |
| DELETE | `/api/tasks/{id}`   | removes one task                                |
| DELETE | `/api/tasks`        | removes the completed ones                      |
| POST   | `/api/signups`      | records a signup                                |
| GET    | `/api/inventory`    | stock, `?search=&sort=&direction=`              |

## Configuration

| Variable       | Required | Meaning                         |
| -------------- | -------- | ------------------------------- |
| `DATABASE_URL` | yes      | PostgreSQL connection string    |
| `API_TOKEN`    | yes      | the bearer token `/api` expects |
| `PORT`         | no       | defaults to 8080                |

## Divergences from the Go, C# and Java siblings

Each one is a place where Rust's answer is genuinely different, rather than a
place where this repository disagrees about the standard.

- **The layer boundary is a compile error.** Standard 1. Go asserts it with a
  test and Java with ArchUnit. Cargo refuses it outright, so the only thing left
  to assert is which crates a layer may *list*.

- **No regex for the email check.** Go, C# and Java all use one because their
  standard libraries ship a regex engine. Rust's does not, and taking a regex
  crate into the domain to answer three questions would cost a dependency to say
  less clearly what is accepted. The check is written out.

- **The domain takes a dependency for a UUID.** All three siblings get one from
  their standard library. `uuid` is a value type that reaches nothing, so the rule
  the domain holds to still holds, but it is a dependency and the conventions
  crate asserts the list stays at two.

- **The ports block, where the C# ones are async.** The C# sibling returns `Task`
  and threads a `CancellationToken` through every signature. Java 21 runs each
  request on a virtual thread and blocks. Rust has neither: an async port would
  have to choose a runtime in the `application` crate and put a boxed future or
  `async_trait` in every signature, which is a framework decision reaching into
  the layer that exists to be free of them. The runtime is a fact only at the
  edge, and blocking work gets a thread there.

- **An enum where the siblings hold text.** `Plan` is an enum here. The siblings
  keep the submitted string so they can tell an absent plan from an unrecognized
  one; Rust needs no such trick, because the constructor returns which of the two
  failures occurred and the type that results can only be a plan on offer.

- **A match with no default arm, without needing to be asked.** Java needs a
  `switch` with no `default` and a sealed hierarchy to get what Rust's
  exhaustiveness gives by default. Adding an event variant stops the build at the
  outbox's wire-name function until it has a name.

- **No framework at the edge.** The C# sibling uses Minimal APIs because ASP.NET
  Core *is* the platform there. Go uses `net/http` and Java `jdk.httpserver`,
  both from their standard libraries. Rust ships no HTTP server at all, so this
  takes `hyper`, which is the closest thing to the platform, and writes the
  routing. A framework would supply its own opinions about routing, binding and
  error mapping, and those opinions are what this repository exists to state for
  itself. The conventions carry over to axum unchanged; what would not carry over
  is a reader's ability to see them.

- **The dependency floor is higher than Go's, irreducibly.** `go-standards` has
  exactly one direct dependency, `pgx`, because its standard library ships an
  HTTP server, a JSON codec and a scheduler. Rust ships none of those. Counting
  them is fair; pretending the difference is a standards failure is not.

- **Percent-decoding and constant-time comparison are written out.** Fifteen
  lines and six lines respectively. The alternative is two dependencies at the
  edge for two functions, and a reader of a standards repository should be able to
  see what both do.

- **cargo-deny rather than three tools.** It answers the advisory, licence and
  source questions from one reading of the lockfile. Three tools reading it
  separately is three chances for them to disagree about what is in the build.

## Adopting this

Take the standards and the `conventions` crate. The three domains here (tasks,
signups, stock) exist so the structure has something to hold, and they match the
Go, C# and Java siblings so the four can be compared line for line. They are not
the point.
