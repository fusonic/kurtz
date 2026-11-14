# Contributing to /kurtz

Thanks for helping. This guide gets a local dev stack running and explains the
conventions the CI checks.

## Requirements

- Docker with Compose and `make`. That is enough to run the dev stack.
- For running tests and lints on your machine: Rust 1.85 or newer (`rustup`,
  with `rustfmt` and `clippy`) and `curl`.

## Local dev stack

```sh
make start   # build and run the app with PostgreSQL + MySQL in Docker (foreground)
make stop    # stop everything; data and build cache are kept
```

Open <http://localhost:8080/admin> and log in with the password `dev`.

The app container runs a one-time **debug build** with `LOG_LEVEL=debug`, so every
request is logged. There is no file watcher: stop it with Ctrl+C and run
`make start` again after a change (the build cache makes that quick). Migrations
run on each start. The first build takes a few minutes.

| Command | What it does |
| --- | --- |
| `make start-prod` | Build the release image and run it against the dev databases (needs port 8080 free) |
| `make start DB=mysql` | Run the app against MySQL instead of PostgreSQL |
| `make reset` | Stop and delete database data and the build cache |
| `make db` | Start only the databases |
| `make run` | Run the app natively instead of in Docker (needs `make db`) |
| `make help` | List all targets |

Settings for the app container are in [`compose.dev.yml`](compose.dev.yml);
`KURTZ_DEV_LOG_LEVEL` overrides the log level. Kurtz answers on any hostname, so
<http://localhost:8080> and <http://127.0.0.1:8080> both work.

Sample data: with the app running, `make seed` imports [`dev/demo.csv`](dev/demo.csv) (20 links).
You can also upload that file through **Import** in the admin.

## Tests

```sh
make test
```

All tests live in [`tests/`](tests). Tests that need a database run once per
configured backend. `make test` points them at the `kurtz_test` databases of the
dev stack, which the tests **wipe**; never point `KURTZ_TEST_POSTGRES_URL` or
`KURTZ_TEST_MYSQL_URL` at data you care about. Without those variables the
database tests are skipped and say so.

CI runs the suite against PostgreSQL plus both MySQL 9 and MariaDB 12.

## Styles and templates

Templates are [Askama](https://github.com/askama-rs/askama) files in
[`templates/`](templates); they are compiled into the binary, so template errors
are compile errors. Styling is [Tabler](https://tabler.io): no build step, no Node.
The version and the SRI hashes of its stylesheet and script (Bootstrap tooltips)
are pinned in [`assets/tabler.lock`](assets/tabler.lock).
In development the pages load both from the jsDelivr CDN with those hashes as
`integrity`; the Docker build downloads the same files, `build.rs` verifies them
against the hashes and the binary embeds them. To upgrade, change the version
and both hashes in the lock file (the file explains how to compute them).

JavaScript is optional enhancement only: every page must keep working with
JavaScript disabled. htmx is vendored in `assets/vendor/`.

## Database changes

- Add a new migration file to **both** `migrations/postgres/` and
  `migrations/mysql/`, with the same timestamp prefix.
- Never edit a migration that has been released; sqlx checks their checksums.
- Write queries once with `$1, $2, …` placeholders, each used once and in order.
  `src/db.rs` converts them for MySQL. Keep SQL portable; that module is the only
  place that may know about dialect differences.

## Before you open a pull request

```sh
make lint
make test
```

- Every environment variable the code reads must be documented in the README;
  a test enforces this.
- Keep dependencies few. New crates need a reason in the pull request.

## Releasing

Maintainers push a tag:

```sh
git tag v1.2.0 && git push origin v1.2.0
```

The release workflow runs all tests, builds a multi-arch image (amd64, arm64),
pushes it to the GitHub Container Registry (`ghcr.io/fusonic/kurtz`) as `1.2.0`,
`1.2` and `latest` and creates a GitHub Release with generated notes.
Pre-release tags such as `v1.3.0-rc1` only get their own tag.

No extra secrets are needed: the workflow pushes with the built-in `GITHUB_TOKEN`.
After the first release, set the package's visibility to public in the repository's
package settings.
