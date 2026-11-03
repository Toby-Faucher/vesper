-- Passkeys (issue #151). Additive only.

-- One row per registered passkey (WebAuthn credential). credential_id is the authenticator's id
-- (unique across accounts, so a sign in finds the row from it). credential is the webauthn-rs
-- `Passkey` as JSON: the public key and the signature counter, no private key. Deleting the
-- account deletes its passkeys. Resetting the password or banning leaves them; sign in checks the ban.
CREATE TABLE passkeys (
    id INTEGER PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    credential_id BLOB NOT NULL UNIQUE CHECK (length(credential_id) BETWEEN 1 AND 1024),
    credential TEXT NOT NULL,
    name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 64),
    created_at INTEGER NOT NULL,
    last_used_at INTEGER NULL
) STRICT;

CREATE INDEX passkeys_account ON passkeys (account_id);
