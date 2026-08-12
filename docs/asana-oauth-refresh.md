# Refresh Asana OAuth for interactive backlog creation

The `asana-backlog` skill uses a short-lived OAuth access token. Keep the OAuth
client secret and refresh token out of Codex and the long-running Factory
daemon. A systemd oneshot service can decrypt those two credentials, exchange
the refresh token, verify the resulting access token, and write only that
short-lived token to a private runtime file.

Repeat this setup for every project-specific launcher. The example service name
is `example-project`; replace it consistently with a short, filesystem-safe
name. All IDs shown are placeholders, not real values.

## Prerequisites

Complete the Asana [authorization-code flow with
PKCE](https://developers.asana.com/docs/oauth) for an app with Full Permissions
disabled and exactly these scopes:

```text
tasks:read
tasks:write
projects:read
tags:read
custom_fields:read
```

The authorization exchange returns an access token and long-lived refresh
token. Keep the client secret and refresh token available only long enough to
encrypt them below. The initial access token is disposable because the service
will mint and verify a replacement.

## 1. Encrypt the refresh credentials

Run these commands in Bash. The silent prompts keep both secrets out of shell
history:

```bash
sudo install -d -m 0700 /etc/credstore.encrypted

read -rsp "Asana OAuth client secret: " ASANA_CLIENT_SECRET
printf '\n'
printf '%s' "$ASANA_CLIENT_SECRET" | sudo systemd-creds encrypt \
  --name=asana_oauth_client_secret \
  - \
  /etc/credstore.encrypted/factory-example-project-asana-oauth-client-secret.cred
unset ASANA_CLIENT_SECRET

read -rsp "Asana OAuth refresh token: " ASANA_REFRESH_TOKEN
printf '\n'
printf '%s' "$ASANA_REFRESH_TOKEN" | sudo systemd-creds encrypt \
  --name=asana_oauth_refresh_token \
  - \
  /etc/credstore.encrypted/factory-example-project-asana-oauth-refresh-token.cred
unset ASANA_REFRESH_TOKEN
```

Do not put either secret in an environment file, launcher, repository, task,
command argument, or systemd unit. A warning that the systemd host credential
key is not on encrypted media means full-disk encryption would provide stronger
offline physical protection; it does not mean the credential was written as
plaintext.

## 2. Install the refresh helper

Save this as
`/usr/local/libexec/factory-example-project-asana-oauth-refresh`, owned by root
and mode `0755`. Replace no values in the script; its non-secret client ID and
output path come from the service environment:

```python
#!/usr/bin/env python3
import json
import os
import tempfile
import urllib.parse
import urllib.request

EXPECTED_SCOPES = {
    "custom_fields:read",
    "projects:read",
    "tags:read",
    "tasks:read",
    "tasks:write",
}
TOKEN_URL = "https://app.asana.com/-/oauth_token"
TOKEN_INFO_URL = "https://app.asana.com/-/token_info"


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(
        self, request, file_pointer, code, message, headers, new_url
    ):
        raise RuntimeError(
            "refusing to forward Asana credentials through a redirect"
        )


def read_credential(name):
    directory = os.environ.get("CREDENTIALS_DIRECTORY")
    if not directory:
        raise SystemExit("CREDENTIALS_DIRECTORY is not set")
    with open(os.path.join(directory, name), encoding="utf-8") as handle:
        value = handle.read().strip()
    if not value:
        raise SystemExit(f"credential {name} is empty")
    return value


def post_form(url, values):
    request = urllib.request.Request(
        url,
        data=urllib.parse.urlencode(values).encode(),
        method="POST",
        headers={
            "Content-Type": "application/x-www-form-urlencoded",
            "User-Agent": "flashy-factory-asana-oauth-refresh/1",
        },
    )
    opener = urllib.request.build_opener(NoRedirect)
    with opener.open(request, timeout=30) as response:
        return json.load(response)


def write_token(path, token):
    descriptor, temporary_path = tempfile.mkstemp(
        prefix=".asana-token-", dir=os.path.dirname(path)
    )
    try:
        os.fchmod(descriptor, 0o600)
        os.write(descriptor, token.encode())
        os.fsync(descriptor)
        os.close(descriptor)
        descriptor = -1
        os.replace(temporary_path, path)
    finally:
        if descriptor >= 0:
            os.close(descriptor)
        try:
            os.unlink(temporary_path)
        except FileNotFoundError:
            pass


def main():
    client_id = os.environ["ASANA_OAUTH_CLIENT_ID"]
    output_path = os.environ["ASANA_OAUTH_ACCESS_TOKEN_PATH"]
    tokens = post_form(
        TOKEN_URL,
        {
            "grant_type": "refresh_token",
            "refresh_token": read_credential("asana_oauth_refresh_token"),
            "client_id": client_id,
            "client_secret": read_credential("asana_oauth_client_secret"),
        },
    )
    access_token = tokens.get("access_token")
    if not isinstance(access_token, str) or not access_token:
        raise SystemExit("Asana did not return an access token")

    info = post_form(TOKEN_INFO_URL, {"token": access_token})
    if info.get("active") is not True or info.get("token_type") != "bearer":
        raise SystemExit("Asana returned an inactive or non-bearer token")
    if str(info.get("client_id", "")) != client_id:
        raise SystemExit("Asana returned a token for another OAuth app")
    if set(str(info.get("scope", "")).split()) != EXPECTED_SCOPES:
        raise SystemExit("Asana returned an unexpected OAuth scope set")
    if float(info.get("expires_in", 0)) < 3000:
        raise SystemExit("Asana returned an unexpectedly short token lifetime")

    write_token(output_path, access_token)
    print("Refreshed and verified the Asana OAuth access token.")


if __name__ == "__main__":
    main()
```

Syntax-check it before installing the units:

```sh
python3 -m py_compile \
  /usr/local/libexec/factory-example-project-asana-oauth-refresh
```

## 3. Install the service and timer

Create
`/etc/systemd/system/factory-example-project-asana-oauth-refresh.service`.
Replace the Unix account, group, and OAuth client ID. The client ID is an
identifier, not a secret:

```ini
[Unit]
Description=Refresh Asana OAuth for example-project backlog creation
Wants=network-online.target
After=network-online.target

[Service]
Type=oneshot
User=<unix-user>
Group=<unix-group>
Environment=ASANA_OAUTH_CLIENT_ID=<oauth-client-id>
Environment=ASANA_OAUTH_ACCESS_TOKEN_PATH=/run/factory-example-project/asana-oauth-access-token
LoadCredentialEncrypted=asana_oauth_client_secret:/etc/credstore.encrypted/factory-example-project-asana-oauth-client-secret.cred
LoadCredentialEncrypted=asana_oauth_refresh_token:/etc/credstore.encrypted/factory-example-project-asana-oauth-refresh-token.cred
RuntimeDirectory=factory-example-project
RuntimeDirectoryMode=0700
RuntimeDirectoryPreserve=yes
ExecStart=/usr/local/libexec/factory-example-project-asana-oauth-refresh
NoNewPrivileges=yes
PrivateTmp=yes
```

Create `/etc/systemd/system/factory-example-project-asana-oauth-refresh.timer`:

```ini
[Unit]
Description=Keep example-project Asana OAuth access fresh

[Timer]
OnBootSec=2min
OnUnitActiveSec=45min
AccuracySec=1min
Persistent=yes
Unit=factory-example-project-asana-oauth-refresh.service

[Install]
WantedBy=timers.target
```

The timer refreshes before Asana's typical one-hour access-token expiry. The
service owns the private runtime directory as the interactive Unix user and
writes the token atomically as mode `0600`.

## 4. Start and verify refresh

Load the units, run one refresh immediately, then enable the timer:

```sh
sudo systemctl daemon-reload
sudo systemctl start factory-example-project-asana-oauth-refresh.service
sudo systemctl enable --now factory-example-project-asana-oauth-refresh.timer

systemctl show factory-example-project-asana-oauth-refresh.service \
  -p Result -p ExecMainStatus
systemctl list-timers factory-example-project-asana-oauth-refresh.timer
stat -c '%a %U:%G %n' \
  /run/factory-example-project \
  /run/factory-example-project/asana-oauth-access-token
```

Expect `Result=success`, `ExecMainStatus=0`, a `0700` runtime directory, and a
`0600` token owned by the configured Unix user. Do not print or inspect the
token itself.

Finally, install the [project-specific Codex launcher](asana.md#install-a-project-specific-codex-launcher).
Its `token_file` path must exactly match `ASANA_OAUTH_ACCESS_TOKEN_PATH` above.
The normal SSH command is then:

```sh
codex-example-project
```

If refresh fails, inspect metadata-only status first:

```sh
systemctl status factory-example-project-asana-oauth-refresh.service
journalctl -u factory-example-project-asana-oauth-refresh.service --since today
```

Reauthorize the Asana app and replace the encrypted refresh token if the user
revokes access, the refresh token becomes invalid, the app's scopes change, or
the app's client secret is rotated.
