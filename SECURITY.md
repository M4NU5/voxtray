# Security

## Reporting a vulnerability

Please report privately through GitHub: **Security > Report a vulnerability** on
https://github.com/harshaldevvv/voxtray. Do not open a public issue for it. You should get a reply within a week.

## Design notes

- API keys live only in the OS keychain and are sent only to the provider they belong to, over HTTPS.
- Voxtray opens no network ports. It only makes the outgoing requests listed in [PRIVACY.md](PRIVACY.md).
- Release builds are produced by GitHub Actions from the tagged source.
