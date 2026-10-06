# Hosting a ranked lobby

For hosts of Genji Ball Ranked. You run the host tool next to Overwatch; it uploads your ranked matches by itself.

## Set up (once)

1. **Install the tool.** Download the `.exe` from the [Releases page](https://github.com/Genji-Ball-Team/genjiball-host-tool/releases) and run it. Windows will warn about an unknown app (the installer isn't code-signed): click "More info", then "Run anyway".
2. **Get a host token.** Ask a genjiball.us admin for one. Admins make them at <https://genjiball.us/admin>. The token is like a password for your uploads: don't share it.
3. **Paste the token** into the "Host token" box and click "Check and save". The tool checks it with the server and keeps it on your PC, encrypted for your Windows account. Once it's accepted you'll see your host name.
4. **Pick your region**, EU or NA. Ranked keeps the two apart, so pick the one you're about to host in. If you host in the other region another night, switch it first. The tool shows the region it uploads as, under "Uploads".
5. **Turn on the Workshop log file in Overwatch.** Overwatch only writes the log the tool reads if **Enable Workshop Inspector Log File** is on (Options, Gameplay, General). Without it, nothing is uploaded.
6. **Check the log folder.** The tool looks in `Documents\Overwatch\Workshop` and says "Found automatically". If it says the folder isn't there yet, that's normal until Overwatch has written its first log. If your Documents folder is somewhere else, or the logs go elsewhere, click "Choose folder..." and pick it.

The tool updates itself: when a new version is out, a banner at the top offers "Install and restart" (or use "Check for updates"). If you have v0.1.0, which can't, download the newest installer by hand once.

You can close the window afterwards: the tool keeps uploading from the tray (next to the clock). Quit it from the tray icon's menu.

## Every time you host

1. Open the tool. Check the region is the one you're hosting in.
2. Click **Copy ranked code**. The tool builds the code for the latest ranked version of Genji Ball and copies it. The top 10 players of your region are tagged with their place and rating, and everyone else with their rank tier. The tags are as new as the code, so copy a fresh one each time you host.
3. In Overwatch, create a new custom game, open Settings and import the code with the "Import / paste settings" button (top right), as in the [Genji Ball setup steps](https://github.com/Genji-Ball-Team/GenjiBall-CE#readme). Importing on top of an existing custom game can fail. Start the lobby as usual and host the game.
4. Play. Ranked matches are uploaded when they end. If you move to spectator, close the lobby or Overwatch crashes mid-match, the tool uploads what's in the log once it stops growing.

Only matches played after you first saved a token for a server are uploaded to that server.

## Your lobby on the site

While you host a ranked match, the tool lists your lobby on genjiball.us, under your region, with how many players are in it, so players can find it. Under "Live lobby" in the tool you can give it a name (optional, shown next to your host name), or untick the box to stop listing it. The tool says whether it's listed right now.

It's taken off the list when the match ends, when the log stops growing (you closed the lobby or Overwatch closed), and when you quit the tool. If the tool can't tell the server (you went offline, say), the site drops it by itself after a few minutes. A match the game marks UNRANKED isn't listed.

## Going AFK

If you have to step away but want to keep the lobby going, click **Go AFK** at the top of the tool. You keep your slot, and your rating doesn't change in rounds that start while AFK is on: the server leaves you out of them, as if you had left. Other players still count, and are rated on their order without you.

- A round already under way when you click still counts for you. AFK starts with the next round.
- AFK stays on across matches, and when you restart the tool, until you click **I'm back: turn AFK off**. While it's on, the AFK section is red and the header says **AFK**.
- The AFK rounds go up with the match's upload. The tool shows which rounds of the latest match weren't rated for you so far.

## The upload list

Under "Uploads" each log file is listed with the players in it and what happened to it.

| It says | What it means |
|---|---|
| Being played: uploaded when the match ends or the log stops growing | The game is still writing this file |
| Waiting to upload | Ready, will go in the next moments |
| Upload failed: ... Retrying automatically | No connection or the server had a problem. It tries again by itself; "Retry now" tries at once |
| Accepted | The match is stored and counts. "View on the site" opens its page |
| Waiting for an admin | An admin has to check it before it counts. Either you're an untrusted host (every match waits; ask an admin to make you trusted) or the match has something odd, such as two players with the same name |
| Rejected: ... | The server didn't take the match; the reason is shown |
| Voided by an admin | An admin removed it from the ratings |
| Already uploaded | The server already has this match, nothing more to do |
| No match in it | The file has no ranked match (for example a different game mode) |

The status can change later: an admin may accept, reject or void a match after you uploaded it. "Check again" asks the server straight away.

## The line above the list

| It says | What to do |
|---|---|
| Watching for ranked matches | Nothing, all good |
| Watching. N ranked logs to upload | They're on their way |
| Paused: there's no host token for this server | Paste a token |
| Paused: the server doesn't know this token | Check you pasted all of it, or ask an admin for a new one |
| Paused: this token was revoked | Ask an admin for a new one |
| Paused: you have no home region yet | Pick your region |
| Waiting for the Workshop log folder | Choose the log folder, or turn on the log file in Overwatch and play a game |

If something else is wrong, ask in the [Discord](https://discord.gg/sv9VVjh5pT).
