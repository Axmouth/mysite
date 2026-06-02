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
`latest` tag to GHCR, uploads the Compose file over SSH, pulls the new image on
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

The production Compose file joins the shared external Docker network named
`web`. It declares its public hostname with Traefik labels. The shared proxy
must be installed once on the server before deploying the app.

## Shared reverse proxy

The server should run one Traefik stack that owns ports `80` and `443`. Each
app remains in its own repository and declares its own hostname using Docker
labels. Traefik discovers those labels and requests TLS certificates
automatically.

The proxy template is stored in `deploy/proxy`. Copy `compose.yaml` to
`/opt/proxy/compose.yaml` on the server and copy `.env.example` to
`/opt/proxy/.env`. Then install it once from the server:

```sh
docker network create web
sudoedit /opt/proxy/.env
cd /opt/proxy
docker compose up -d
```

Only `ACME_EMAIL` is required in `/opt/proxy/.env`.

When migrating from the older bundled Caddy setup, stop the old app stack
before starting Traefik because both proxies need ports `80` and `443`:

```sh
cd /opt/mysite
SITE_IMAGE=unused docker compose -f compose.deploy.yaml down
```

Then install the shared proxy and deploy this app again. The `site-data` volume
is kept by `docker compose down`. The old Caddy volumes can be removed later
after the migration is confirmed.

For another app, add a production Compose file inside that app's own
repository. For example, a notes app could store this as
`compose.deploy.yaml`:

```yaml
services:
  notes:
    image: ${NOTES_IMAGE:?Set NOTES_IMAGE}
    restart: unless-stopped
    expose:
      - "8080"
    networks:
      - web
    labels:
      - "traefik.enable=true"
      - "traefik.docker.network=web"
      - "traefik.http.routers.notes.rule=Host(`${NOTES_DOMAIN:?Set NOTES_DOMAIN in .env}`)"
      - "traefik.http.routers.notes.entrypoints=websecure"
      - "traefik.http.routers.notes.tls.certresolver=letsencrypt"
      - "traefik.http.services.notes.loadbalancer.server.port=8080"

networks:
  web:
    external: true
```

The labels go under the app service, next to `image`, `expose`, and `networks`.
`expose` is the port that the app listens on inside its container. It does not
publish that port directly on the server. Traefik receives HTTPS traffic and
forwards it to that private port.

Create a server directory and `.env` file for that app:

```sh
mkdir -p /opt/notes
cd /opt/notes
nano .env
```

For this example, `/opt/notes/.env` contains:

```env
NOTES_DOMAIN=notes.axmouth.dev
```

Upload or copy the notes app's `compose.deploy.yaml` to
`/opt/notes/compose.deploy.yaml`, then deploy its image:

```sh
cd /opt/notes
NOTES_IMAGE=ghcr.io/your-user/notes:latest docker compose -f compose.deploy.yaml pull
NOTES_IMAGE=ghcr.io/your-user/notes:latest docker compose -f compose.deploy.yaml up -d
```

The shared Traefik container notices the labels automatically. There is no
central proxy route file to edit and no proxy restart is needed.

Use a unique router and service name such as `notes` for each app. If another
project uses port `8080` inside its own container, that is fine. Add a DNS
record for each subdomain, or add one wildcard `*.axmouth.dev` record pointing
to the server so new subdomains work without further DNS edits.

The basic proxy template mounts the Docker socket read-only so Traefik can
discover containers. Access to the Docker API is security sensitive. For a
more hardened server, put a restricted Docker socket proxy in front of Traefik.

## Configuration

| Variable | Default | Purpose |
| --- | --- | --- |
| `ADMIN_PASSWORD` | required | Password for the private admin area |
| `BIND_ADDRESS` | `127.0.0.1:3000` | Address used by the Rust server |
| `DATA_DIR` | `data` | Directory for SQLite and uploaded images |
| `SITE_URL` | `http://127.0.0.1:3000` | Public base URL used in metadata and the sitemap |
| `SITE_DOMAIN` | `example.com` | Public hostname used by Traefik in production |
| `COOKIE_SECURE` | `false` | Set to `true` when the site is served over HTTPS |
| `PORT` | `3000` | Host port used by Docker Compose |

## Production notes

The production Compose file publishes Traefik labels for HTTPS. Set `SITE_URL`
to the public HTTPS URL, set `SITE_DOMAIN` to the hostname, and set
`COOKIE_SECURE=true`.

The site adds search metadata, Open Graph metadata, `robots.txt`, and
`sitemap.xml`. The site title, author name, description, social image, copyright
claim, and footer links can be edited from `/admin/settings`.

The admin area has same-origin checks for changes, login throttling, secure
response headers, and upload validation. Raw HTML in Markdown is ignored.

The `data` volume contains both the SQLite database and uploaded images. Keep
that volume when replacing or upgrading the container.
