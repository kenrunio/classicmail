# Security

ClassicMail handles people's mail, so security reports are very welcome.

## Reporting a problem

Please **don't open a public issue** for a security problem. E-mail **info@duvz.com** instead, with
what you found, the steps to reproduce it and the ClassicMail version (Help › About). You'll get a
reply within a few days, and credit in the changelog if you'd like it.

## How ClassicMail keeps your data

- There is no ClassicMail server. The app talks only to Google (and, if you use the AI features,
  to the AI provider whose key you entered), over HTTPS.
- Each user signs in through their **own** Google Cloud project ([docs/google-setup.md](docs/google-setup.md)).
- Google sign-ins and AI API keys are kept in Windows Credential Manager, not in files.
- Synced data is stored in the user's profile (`%APPDATA%\com.ekgir.classicmail`). Protect it like
  other files, for example with BitLocker.
- Logs record technical events only, not message content.

## Supported versions

Only the latest release gets fixes.
