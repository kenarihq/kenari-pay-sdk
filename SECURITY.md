# Security

kenari-pay-sdk checks what a merchant is about to trust with money: the address a customer pays
to, and the webhook that says a deposit arrived. A flaw in either is a security issue.

## Reporting a vulnerability

Write to **security@kenari.id**. Please do not open a public issue or pull request for it.

Say which version or commit you tested, what the flaw lets an attacker do, and how to reproduce it,
with a failing test if you can. We acknowledge a report within a few working days and tell you when
it is fixed. If you want credit, say how you want to be named.

## What counts

- An address that passes `address::verify` and does not pay a wallet the merchant listed, or that
  was not made from the factory and implementation the merchant pinned.
- A webhook that passes `webhook::verify` without a valid signature, a stale timestamp that is
  accepted, or a secret that is empty or wrong and still verifies.
- A pool address that passes without a valid attestation by the merchant's key.
- A secret, such as the API key, that reaches a log or an error text.
- A panic reachable from input the gateway or a third party controls.

## What does not

The gateway itself, the contracts and the operator console are in the kenari-pay repository and
are reported to the same address. A merchant's own handling of a verified deposit, crediting it
twice for one, say, is the merchant's code and is not covered here.

## Supported versions

The latest release and the main branch.
