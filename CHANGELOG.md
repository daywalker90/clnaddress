# Changelog

## [0.1.5] - 2026-08-15

### Added
- `clnaddress-nostr-privkey-file` option to read the nostr private key from a file, so the key is not exposed in the config or `listconfigs`

### Changed
- `clnaddress-nostr-privkey` is now deprecated, existing setups keep working; `clnaddress-nostr-privkey-file` takes precedence when both are set. When only the deprecated option is set, the key is automatically migrated to `<lightning_dir>/clnaddress/nostr-secret-key` on startup, so the option can be removed afterwards
- LNURL error responses now return HTTP 200 with `status: "ERROR"` in the body (LUD-06) instead of HTTP error status codes, so wallets can parse them consistently
- `clnaddress-listuser` is now case sensitive, matching `clnaddress-adduser` and `clnaddress-deluser`
- zap receipts are now sent to the first 10 relays listed in the zap request, in a separate task with a lower timeout, so slow or unreachable relays no longer delay receipt processing

### Fixed
- validation of the optional `P` tag in zap requests
- `users.json` and `payindex.json` are now written atomically to avoid corruption on a crash

## [0.1.4] 2026-03-29

### Added
- release binaries for macOS

## [0.1.3] 2026-03-29

### Changed
- updated `cln-rpc` and `cln-plugin` dependencies
- MSRV raised to 1.85

## [0.1.2] 2025-06-11

### Added
- `clnaddress-listuser` to list the users and their settings

### Changed
- The minimum `clnaddress-min-receivable` is now 0 and also defaults to 0 (any amount allowed). Some services "validate" a lightning address by trying to call the callback with ``amount=0`` which they shouldn't when the minimum is `>0`

### Fixed
- User names with only numbers
- Descriptions with only numbers

## [0.1.0] 2025-03-25

### Added

- initial release of `clnaddress`
