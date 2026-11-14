<p align="center">
  <img src="docs/logo-light.svg#gh-light-mode-only" alt="/kurtz" width="440">
  <img src="docs/logo-dark.svg#gh-dark-mode-only" alt="/kurtz" width="440">
</p>

<p align="center"><strong>Modern, high performance URL shortener for self-hosting, built with Rust.</strong><br>
GDPR compliant: no IP addresses or other user data are logged or stored.</p>

<p align="center">
  <a href="https://github.com/fusonic/kurtz/releases/latest"><img src="https://img.shields.io/github/v/release/fusonic/kurtz?sort=semver&label=release" alt="Latest release"></a>
  <a href="https://github.com/fusonic/kurtz/pkgs/container/kurtz"><img src="https://img.shields.io/badge/ghcr.io-fusonic%2Fkurtz-blue?logo=docker&logoColor=white" alt="Container image"></a>
  <a href="https://github.com/fusonic/kurtz/actions/workflows/ci.yml"><img src="https://github.com/fusonic/kurtz/actions/workflows/ci.yml/badge.svg?branch=main" alt="Pipeline"></a>
  <a href="https://github.com/fusonic/kurtz/actions/workflows/test.yml"><img src="https://github.com/fusonic/kurtz/actions/workflows/test.yml/badge.svg?branch=main" alt="Tests"></a>
  <a href="https://github.com/fusonic/kurtz/blob/main/LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="MIT license"></a>
</p>

<p align="center">
  <img src="docs/mockup.png" alt="The /kurtz admin: a list of short links with tags, filters and on/off switches" width="960">
</p>

/kurtz turns long URLs into short ones on your own domain. It answers on whatever
hostname it is reached on; to serve several domains with different links, run
several installations.
Links carry tags, can be switched off without deleting them, and come with QR
codes. The admin is a fast, plain web UI behind a single password.

It runs as one static binary in a minimal Docker image and stores data in
PostgreSQL or MySQL. /kurtz serves thousands of requests per second on a regular
machine while keeping memory usage below 16 MB. Redirects are served from memory
in well under a millisecond. No IP addresses, user agents, referrers
or cookies of visitors are logged or stored; only a last-used date per link.
Inspired by
[Chhoto URL](https://github.com/SinTan1729/chhoto-url).

## Features

- Auto-generated slugs (5 characters `a-z0-9`) or your own, case-insensitive.
- CSV import (also from Shlink) and export.
- Query parameters on a short link are passed through to the target.
- Tags with autosuggest; filter and search the list by tag.
- Switch links on and off; duplicate check when a target is already shortened.
- Created, updated and last-used date per link.
- QR codes as SVG and PNG with a transparent background, visible to the admin only.
- `robots.txt` and `noindex` headers keep search engines out.

## Installation

/kurtz speaks plain HTTP on port 8080. It does not handle TLS: put a reverse
proxy or CDN in front of it to serve HTTPS.

You need a PostgreSQL or MySQL database. Then:

```sh
# 1. Hash your admin password (needs the `argon2` package)
echo -n "MY_SECRET" | argon2 "$(openssl rand -hex 16)" -e

# 2. Create a session secret
openssl rand -hex 32

# 3. Run /kurtz
docker run -p 8080:8080 \
  -e DATABASE_URL=postgres://kurtz:password@db-host:5432/kurtz \
  -e ADMIN_PASSWORD='$argon2i$v=19$…' \
  -e SESSION_SECRET=<output of step 2> \
  ghcr.io/fusonic/kurtz:latest
```

Point your domain at your proxy, forward it to port 8080 and open
`https://s.example.com/admin`. Database migrations run automatically on every start.

## Configuration

All settings are environment variables.

| Name | Description | |
| --- | --- | --- |
| `DATABASE_URL` | Database connection: `postgres://user:pass@host:5432/db` or `mysql://user:pass@host:3306/db`. | required |
| `ADMIN_PASSWORD` | Argon2 hash of the admin password (see above). Startup fails if it is not a valid hash. | required |
| `SESSION_SECRET` | Key for signing login cookies, at least 64 characters. Changing it logs you out. | required |
| `SLUG_LENGTH` | Length of auto-generated slugs. Default `5`. | optional |
| `ROOT_REDIRECT_URL` | Where `/` redirects. Without it, `/` returns 404. | optional |
| `COOKIE_SECURE` | Users reach /kurtz over HTTPS (through your proxy): cookies get the `Secure` flag, HSTS is sent and short URLs start with `https://`. Set `false` only for plain-HTTP testing. Default `true`. | optional |
| `CACHE_SIZE` | Number of links kept in the in-memory cache. Default `1000`. | optional |
| `LAST_USED_FLUSH_SECS` | How often last-used dates are written to the database, in seconds. Default `60`. | optional |
| `LOG_LEVEL` | `error`, `warn`, `info` or `debug`. `info` logs startup and every 404; `debug` logs every request. Query strings and client IPs are never logged. Default `info`. | optional |


## How it behaves

**Slugs.** Slugs are unique and case-insensitive. `admin`, `static` and `_health`
are reserved.

**Redirects.** `GET /{slug}` answers `302`. Request parameters are appended to
the target's own query in the order received, and the target's fragment is kept:
`/docs?b=2` with target `https://x.com/p?a=1#top` goes to
`https://x.com/p?a=1&b=2#top`. Disabled links and unknown slugs answer `404`.

**Tags.** Up to 10 per link, lowercase, letters, digits, spaces, `-`, `_` and `.`.
The search box also matches tags.

**Last used.** Each redirect updates a timestamp in memory; /kurtz writes them to
the database every `LAST_USED_FLUSH_SECS` and on shutdown, so no redirect waits
for the database. `HEAD` requests and 404s do not count.

**Cache.** Redirects come from an in-memory LRU cache of `CACHE_SIZE` entries; a
miss costs one indexed query. Every change in the admin clears the cache. Run one
instance.

**Health check.** `GET /_health` answers `200 OK` when the database is reachable
and fully migrated, otherwise `503 NOT_OK`. The reason is only written to the log. It works on any hostname, so
load balancers and uptime monitors can use it; the Docker image's `HEALTHCHECK`
calls it too.

## FAQ

### My admin panel shows the wrong domain, and URLs and QR codes are wrong.

Redirects work on any hostname. The admin builds short URLs and QR codes from the
scheme, host and port the visitor used. Behind a proxy or CDN, /kurtz only sees
what the proxy sends it, which is often the internal hostname and plain HTTP.
Make your proxy pass the original values on.

The host comes from the first of these headers that is set, with `Host` as the
fallback:

1. `X-Forwarded-Host`
2. `Forwarded` (`host=`)
3. `X-Original-Host`
4. `X-Host`

The scheme comes from the first of these:

1. `X-Forwarded-Proto`
2. `Forwarded` (`proto=`)
3. `CloudFront-Forwarded-Proto`
4. `X-Forwarded-Scheme`

Without any of them the scheme is `https`, or `http` if `COOKIE_SECURE=false`.

- **nginx:** add `proxy_set_header Host $host;` and
  `proxy_set_header X-Forwarded-Proto $scheme;` to the `location` that proxies to /kurtz.
- **CloudFront:** it sends no visitor host of its own. Use an origin request policy
  that forwards the `Host` header, or add `X-Forwarded-Host` as a custom origin header.
- **Cloudflare:** passes the visitor's `Host` through unchanged; nothing to do.

These headers are trusted from anyone and only decide which URLs the admin shows,
so run /kurtz behind your proxy.

## Privacy

/kurtz is built to be GDPR compliant without a cookie banner:

- **Visitors** following a short link leave no trace in /kurtz: no IP address,
  user agent, referrer or cookie is logged or stored. Per link only the time of
  the last use is kept, which identifies nobody.
- **Logs** contain method, host, path, status and duration. Query strings and IP
  addresses are never logged, at any log level.
- **The admin** uses one strictly necessary session cookie. For login rate
  limiting, failed attempts are counted in memory for one minute, without any
  IP address; the count is never written to disk or the logs.

Your reverse proxy usually writes its own access logs with IP addresses. Those
are outside /kurtz; configure or disable them to match your privacy policy.

## Out of scope

These are deliberately not part of /kurtz:

- Multiple users and permissions: there is one admin.
- Analytics: no visits, referrers or visitor data are collected.
- Conditional redirects: a slug always points to one target.

## Backup

All data lives in the database: use `pg_dump` or `mysqldump`. The CSV export
from the admin is a portable alternative.

## Development

See [CONTRIBUTING.md](https://github.com/fusonic/kurtz/blob/main/CONTRIBUTING.md) for the local dev stack (`make db`,
`make run`), tests and releases.

## License

[MIT](https://github.com/fusonic/kurtz/blob/main/LICENSE)
