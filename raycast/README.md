# Hopp for Raycast

Call a teammate on Hopp by typing their name in Raycast.

This extension isn't in the Raycast Store. To install it, with Raycast open and the Hopp app from this repo running:

```
cd raycast
npm install
npm run dev
```

Once Raycast lists **Call Teammate**, stop the command with Ctrl+C. The extension stays installed.

## Using it

- Run **Call Teammate**, type a name and press return.
- Give the command a hotkey or alias in Raycast Settings > Extensions > Hopp to skip a step.
- To make a person appear in Raycast's main search, select them, press Cmd+S and save. Typing their name and pressing return then calls them. Raycast may ask you to confirm the first time.

## How it works

Hopp writes `launcher.json` to its app data folder, readable only by you. It holds each teammate's ID, name, avatar and online status, plus a key. It contains no login token. The extension reads that file and opens a `hopp:///call` link carrying the key.

The key is there because any web page can open a `hopp://` link. Without it, a link someone sent you could start a call with your mic on.

Web pages can open Raycast deeplinks too, so a name passed to this command only fills in the list. It rings straight away only for quicklinks this extension made, which carry a token from its local storage. That token is in the saved quicklink, so unlike the key it can reach Raycast Cloud Sync if you use it, but it only works on a Mac where this extension already holds the same token.
