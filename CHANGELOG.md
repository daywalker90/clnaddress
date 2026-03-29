# Changelog

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
