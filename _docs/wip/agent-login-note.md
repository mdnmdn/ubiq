One command tells you for sure:

ls -l ~/.claude/.credentials.json

Missing → that's it.

The fix

am account login exists for exactly this. It runs Claude's own OAuth flow with the home relocated so the Keychain is unreachable, which forces Claude to write the plaintext credential, and captures it:

just am account login claude-code

Then the honest part: Ubiq does not select an account yet. crates/ubiq-host/src/agent.rs builds a RunSpec naming a harness and a folder and nothing else — that is G89, and P5 in the plan is where it gets a real answer. So after that login you would still need the host to pass the account through. Two small pieces, and I'd rather you choose whether to do them now than have me guess.

Meanwhile I made the failure say what it is. Starting a conversation whose run directory received no login now logs a warning under Harness naming the cause, instead of leaving you to read "Not logged in" inside the transcript as though the agent had said it.

Which way do you want to go?

- Wire account selection now — the host reads the library's configured account (settings default, or one named per agent type) and passes it on the RunSpec. This is the beginning of P5 and it is the correct fix.
- A quicker stopgap — the host also accepts a credential straight from the environment (ANTHROPIC_API_KEY / CLAUDE_CODE_OAUTH_TOKEN) when one is set, so you can export a key and keep going while P5 waits.