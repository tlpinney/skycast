# Security

skycast reads data from services you don't control: Open-Meteo, the National Weather
Service, RainViewer and NOAA. A bug that lets that data reach your terminal as escape
sequences, crash skycast, use unbounded memory or write outside its cache directory
(`~/.cache/skycast/`) is a security bug.

## Reporting

Please report security bugs privately through GitHub:
[Report a vulnerability](https://github.com/tlpinney/skycast/security/advisories/new)
(the Security tab, then "Report a vulnerability"). Please don't open a public issue.

Include what you ran (the location and screen), what happened, and if you can, the
response that triggered it.

## Supported versions

Fixes go into the latest release; there are no backports before 1.0.

## Verifying releases

Release binaries come with `SHA256SUMS` and an SSH signature, `SHA256SUMS.sig`. The
signing key is published as a signing key on the maintainer's GitHub account
(`https://api.github.com/users/tlpinney/ssh_signing_keys`), fingerprint
`SHA256:LiOY0+3e/eOQFeUsG7iMovOBJui3yCAXXHlWz48c4uk`. Each release's notes show the
commands to check it.
