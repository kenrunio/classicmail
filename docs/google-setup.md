# Connecting ClassicMail to your Google account (≈15 minutes, once)

ClassicMail talks to Gmail, Calendar, Contacts and Tasks through Google's official APIs. Google
lets an app do that only through a sign-in "client" registered in a Google Cloud project. ClassicMail
doesn't come with a shared one, so **you create your own** — free, about 15 minutes, once. Your mail
then goes straight between Google and your PC: no ClassicMail server, nobody else's project.

You need:

- the Google account you want to read in ClassicMail (a company **Google Workspace** address, or an
  ordinary **@gmail.com** address);
- ClassicMail installed (see the [README](../README.md)), or the source if you build it yourself.

The steps are the same for both kinds of account except step 3, where you choose who may use your
project.

## 1. Create the project

1. Open <https://console.cloud.google.com/> and sign in with the Google account you'll use.
   - Workspace: if your company doesn't let ordinary users create projects, ask your IT admin to do
     these steps (or to give you the "Project Creator" role).
2. Top bar → project picker → **New Project** → name it `ClassicMail` → **Create**, then make sure the
   new project is selected in the top bar.

## 2. Turn on the four APIs

Go to **APIs & Services → Library**, search for each of these and click **Enable**:

- Gmail API
- Google Calendar API
- People API
- Google Tasks API

## 3. Set up the sign-in screen

1. Go to **Google Auth Platform** (older consoles call it **OAuth consent screen**) → **Get started**.
2. App name `ClassicMail`, user support e-mail: your own address → **Next**.
3. **Audience** — this is the one choice that matters:

   | Your account | Choose | Then |
   |---|---|---|
   | Company address on Google Workspace | **Internal** | Nothing more. Only accounts in your company can use the project, Google doesn't review it, and you stay signed in. |
   | Ordinary @gmail.com (no Workspace) | **External** | Internal isn't offered. Finish the steps below, then do step 3.4. |

4. Contact e-mail: your own address → **Next** → agree → **Create**.
5. **@gmail.com only:** go to **Google Auth Platform → Audience** and click **Publish app** →
   **Confirm**, so the status says **In production**.
   - Why: a project left in **Testing** signs you out every 7 days. Publishing it without
     verification is fine for your own use — Google only reviews apps offered to the public, and an
     unverified project is limited to 100 users, far more than you need.
   - Prefer to stay in Testing? Then add your own address under **Audience → Test users**
     (otherwise sign-in is blocked) and expect to sign in again each week.

You don't need to add scopes (permissions) here; ClassicMail asks for them when you sign in.

## 4. Create the client the app uses

1. **Google Auth Platform → Clients → Create client** (or **APIs & Services → Credentials →
   Create credentials → OAuth client ID**).
2. Application type: **Desktop app**, name `ClassicMail Desktop` → **Create**.
3. Click **Download JSON**. You get a file like `client_secret_1234-abc.apps.googleusercontent.com.json`.

Keep that file private: it isn't a password, but it is your project's sign-in. Don't post it, e-mail
it around or commit it to a repository.

## 5. Connect your account in ClassicMail

1. In ClassicMail, on the welcome screen click **Import client file…** and pick the JSON file
   (later: **Tools → Account Settings → Import client file…**).
2. Click **Sign in with Google…** (later: **Account Settings → New… → Google Workspace / Gmail
   account**). Your browser opens Google's sign-in; choose the account.
3. What Google shows next:
   - **@gmail.com:** a page saying **"Google hasn't verified this app"**. That's expected — the
     app is *your own* project. Click **Advanced** → **Go to ClassicMail (unsafe)**.
   - "ClassicMail wants access to…" with checkboxes: leave them **all ticked** and click
     **Continue** / **Allow**. ClassicMail stops with a clear message if a permission is missing.
4. Back in ClassicMail the account appears and starts downloading mail, newest first. A big
   mailbox takes a while the first time — about 45 minutes per 10,000 messages, paced to Google's
   limits — and you can work normally meanwhile.
5. More mailboxes: repeat step 5.2 for each.
   - A **separate account** (for example a shared `info@yourcompany.com` user) signs in like above.
     With an @gmail.com project in Testing, add it as a test user first.
   - An **alias or group** of your own account can't sign in by itself. Add it as a "Send mail as"
     address in Gmail (Settings → Accounts → Send mail as); ClassicMail then offers it in the
     **From** box of every new message.
6. If you tried the demo accounts, remove them: **Account Settings → select → Remove**.

## Installing on another computer

- Install ClassicMail there and use **Import client file…** with the same JSON file, then sign in.
- Or, if you build ClassicMail yourself: copy the JSON file into the project folder as
  `google-client.json` and run `npm run release`. The installer it builds has your client built
  in, so on the other computer you only sign in. **Don't give that installer to other people** —
  they would be signing in through your project. Others should create their own project with this
  guide.

## If something goes wrong

| Message | Fix |
|---|---|
| "Access blocked: ClassicMail can only be used within its organization" | Your project is **Internal** and you signed in with an account outside your Workspace domain. Use an account in that domain, or create a separate **External** project for that account. |
| "Access blocked: ClassicMail has not completed the Google verification process" | Your @gmail.com project is in **Testing** and this account isn't a test user. Publish the project (step 3.5) or add the account under **Audience → Test users**. |
| "Google hasn't verified this app" | Expected with an @gmail.com project. Click **Advanced → Go to ClassicMail (unsafe)**. |
| Asked to sign in again every week | The project is still in **Testing**. Publish it (step 3.5), then **Account Settings → Repair…**. |
| "This app is blocked" / admin policy (Workspace) | Your admin restricts apps. Google Admin console → Security → Access and data control → API controls: internal apps must be trusted (the default), or the admin trusts ClassicMail's client ID. |
| "needs to sign in again" in the status bar | **Account Settings → select the account → Repair…** |
| Google did not return a refresh token | Visit <https://myaccount.google.com/permissions>, remove ClassicMail, then **Repair…**. |

## Taking access away

- **One account:** in ClassicMail, **Tools → Account Settings → select it → Remove**. That deletes
  what ClassicMail stored for it on your PC and withdraws its access to your Google account.
- **From Google's side**, at any time (for example for a lost laptop): open
  <https://myaccount.google.com/permissions>, choose **ClassicMail** and remove its access.
- **The whole project:** in the Google Cloud console, delete the client (Google Auth Platform →
  Clients) or the project. Every ClassicMail sign-in through it stops working.

Your sign-in is stored in Windows Credential Manager (entry `ClassicMail`), never in a file.
