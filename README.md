# mysite

This is a small personal website written in Rust. It has a public homepage,
a project archive, and a private admin area at `/admin`.

The public site uses server rendered HTML. The admin area stores content as
Markdown and uses a local copy of EasyMDE for editing. Images, footer links,
site metadata, and project publishing can all be managed from the admin area.

## Run locally

Install Rust, then run:

```sh
ADMIN_PASSWORD=choose-a-long-password cargo run
```

Open `http://127.0.0.1:3000`.

The site creates `data/site.db` and `data/uploads/`. The `data` directory is
ignored by Git because it contains local content.

## Run with Docker Compose

Create your local environment file:

```sh
cp .env.example .env
```

Set a long random `ADMIN_PASSWORD`, then start the site:

```sh
docker compose up --build -d
```

Docker stores the SQLite database and uploaded images in the `site-data`
volume. The container health check uses `/healthz`.

The local Compose file has a development-only password fallback so commands
such as `docker compose ps` and `docker compose down` keep working if `.env` is
missing. Set your own password before using the admin area. The production
Compose file always requires an explicit password.

## Run checks

Run the complete local verification suite with:

```sh
scripts/verify.sh
```

It checks formatting, parses the browser scripts, runs `cargo check`, runs the
Rust tests, and starts a temporary server for HTTP smoke tests.

The smoke tests cover health checks, security headers, search metadata,
`robots.txt`, `sitemap.xml`, origin checks, admin login, settings changes,
project publishing, Markdown rendering, image uploads, and local editor assets.

## Continuous integration

GitHub Actions runs the verification suite for pull requests and pushes to
`main`. The workflow is stored in `.github/workflows/ci.yaml`.

The deployment workflow is stored in `.github/workflows/deploy.yaml`. After
verification passes, it builds one Docker image, pushes a commit tag and a
`latest` tag to GHCR, uploads the Compose files over SSH, pulls the new image on
the server, recreates the service, and checks `/healthz`.

Configure these GitHub Actions secrets:

| Secret | Purpose |
| --- | --- |
| `DEPLOY_SSH_HOST` | Server hostname or IP address |
| `DEPLOY_SSH_PORT` | SSH port, usually `22` |
| `DEPLOY_SSH_USER` | SSH user allowed to run Docker Compose |
| `DEPLOY_SSH_KEY` | Private SSH key for deployment |
| `DEPLOY_KNOWN_HOSTS` | Output from `ssh-keyscan` for the server |

Configure these GitHub Actions variables:

| Variable | Purpose |
| --- | --- |
| `DEPLOY_PATH` | Server directory for the Compose files, such as `/opt/mysite` |
| `DEPLOY_HEALTH_URL` | Public site URL used after deployment |

Create `.env` inside `DEPLOY_PATH` on the server before the first deployment.
For example, use `/opt/mysite/.env` when `DEPLOY_PATH` is `/opt/mysite`. Use the
same fields as `.env.example`. Set `SITE_URL` to the public HTTPS URL, set
`SITE_DOMAIN` to the public hostname without `https://`, and set
`COOKIE_SECURE=true`.

The server must have Docker with the Compose plugin. If the GHCR package is
private, run `docker login ghcr.io` once on the server with a read-only package
token.

The production Compose file runs Caddy in front of the Rust service. Caddy
listens on ports `80` and `443`, requests TLS certificates automatically, and
forwards traffic to the private app container. Its certificate data is stored
in the persistent `caddy-data` volume.

## Configuration

| Variable | Default | Purpose |
| --- | --- | --- |
| `ADMIN_PASSWORD` | required | Password for the private admin area |
| `BIND_ADDRESS` | `127.0.0.1:3000` | Address used by the Rust server |
| `DATA_DIR` | `data` | Directory for SQLite and uploaded images |
| `SITE_URL` | `http://127.0.0.1:3000` | Public base URL used in metadata and the sitemap |
| `SITE_DOMAIN` | `example.com` | Public hostname used by Caddy in production |
| `COOKIE_SECURE` | `false` | Set to `true` when the site is served over HTTPS |
| `PORT` | `3000` | Host port used by Docker Compose |

## Production notes

The production Compose file includes Caddy for HTTPS. Set `SITE_URL` to the
public HTTPS URL, set `SITE_DOMAIN` to the hostname, and set
`COOKIE_SECURE=true`.

The site adds search metadata, Open Graph metadata, `robots.txt`, and
`sitemap.xml`. The site title, author name, description, social image, copyright
claim, and footer links can be edited from `/admin/settings`.

The admin area has same-origin checks for changes, login throttling, secure
response headers, and upload validation. Raw HTML in Markdown is ignored.

The `data` volume contains both the SQLite database and uploaded images. Keep
that volume when replacing or upgrading the container.
