# Getting help with CachyPlay

## A game won't run

1. Open CachyPlay and pick the game. The top of the page says what happened, why, and what to do.
2. Press **Fix it** if it's offered. After you play, CachyPlay asks whether it worked: answer it, so
   the next suggestion is better.
3. Still stuck? Open the game's **History** tab: **Tell the developer** has a short text to post in
   the game's Steam discussions, and **Report a problem** saves a zip with the logs.

## CachyPlay itself misbehaves

1. Settings → **Advanced** → **Test CachyPlay's setup**. It checks that CachyPlay is installed,
   can read Steam, and that its launch options are intact.
2. **Help (?) → Report a problem**: type what happened, press **Save to file**, then **Open on
   GitHub** and attach the zip. Your home folder, user name, computer name, Steam IDs, IP
   addresses and tokens are replaced before it's saved.
3. From a terminal, `cachyplay doctor` prints the same checks.

## What to expect

- This is a hobby project in beta. Issues are read, but there's no guaranteed reply time.
- A fix that only helps one game on one PC may not become a built-in fix. Use **Share that this
  worked** on a fix that helped: that's how fixes get marked as tested.

## Known limitations

- **Flatpak Steam** isn't supported yet: CachyPlay can read it but won't change anything.
- **Anti-cheat**: some online games block Linux (PUBG, Rainbow Six Siege, Destiny 2, GTA Online,
  Battlefield 2042). No setting can change that; see areweanticheatyet.com.
- CachyPlay can only change things while **Steam is closed**, because Steam rewrites its settings
  when it exits.
- Updates come from **GitHub Releases**; while the repository is private, the GitHub CLI (`gh`) must
  be signed in.
