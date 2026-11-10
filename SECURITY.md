# Security Policy

## Supported versions

The project is pre-release. There is no released version yet, so there is nothing to backport fixes to.

## Reporting a vulnerability

**Do not open a public issue.** Email <git@sovit.xyz> with:

- a description of the issue and its impact
- steps to reproduce, or a proof of concept
- affected commit or version
- any suggested remediation

You should get an acknowledgement within 7 days. Please allow a reasonable window for a fix before disclosing
publicly. Reporters are credited in the release notes unless they ask not to be.

## Scope

In scope: anything in this repository, including design weaknesses in [ADR 0001](docs/decisions/0001-architecture.md)
(for example ways to receive video without paying, to make a viewer overpay, or to link a viewer to what they
watch). Weaknesses in El Tor itself belong upstream at <https://github.com/el-tor>; tell us too if they affect
nutflix-eltor.

Out of scope: the hosting infrastructure, and issues that require an already-compromised machine or physical access.

El Tor is pre-1.0 and has had no independent security review. Do not use nutflix-eltor with funds you cannot afford
to lose.
