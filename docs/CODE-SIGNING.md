# Code signing

## Code signing policy

Windows installers of Tequila are to be signed for free by
[SignPath.io](https://about.signpath.io), with a certificate by the
[SignPath Foundation](https://signpath.org).

Until that is in place, the installers on the Releases page are unsigned,
and Windows says "Windows protected your PC" the first time one is run
(README, "The first time you open it").

What is signed: the Windows installer (`Tequila-Setup-<version>.exe`) and the
update installer (`Tequila-Update-<from>-to-<to>.exe`). They are built by the
repository's own GitHub Actions workflow
([`release-windows.yml`](../.github/workflows/release-windows.yml)), from this
repository's source at the release's commit. Nothing built on a personal
computer is signed.

**Team roles**

- Committers and reviewers: [ifpintac](https://github.com/ifpintac)
- Approvers: [ifpintac](https://github.com/ifpintac)

Every signing request is approved by hand, for a published release only.

**Privacy policy.** This program will not transfer any information to other
networked systems unless specifically requested by the user or the person
installing or operating it. The connections Tequila makes are the ones its
user sets up:
- the Assistant, with the user's own API key, sending what the user asks
  it;
- render-farm machines the user adds;
- the scripting channel, on the user's own computer (localhost).

- GitHub's releases page, once a day when Tequila starts, to see whether a
  newer version is out (Help > Check for updates turns this off). Only
  GitHub's public release list is fetched; nothing about the user or their
  work is sent.

Tequila sends no usage data.

## How it works

1. A release is published from a commit on `main`.
2. **Actions > Windows release > Run workflow**, with the release's tag,
   builds the installer on GitHub's Windows machine:
   - Rust;
   - the STEP reader against OpenCASCADE's release build;
   - Cycles from source;
   - Inno Setup.

   It keeps the package as an artifact for the next update installer
   (`patch_from_run`).
3. With the `SIGNPATH_API_TOKEN` secret and the `SIGNPATH_ORGANIZATION_ID`
   variable set, the workflow sends the installers to SignPath. It waits
   for the request's approval, downloads the signed files, and attaches them
   to the release.
4. SignPath signs what
   [`.signpath/artifact-configuration.xml`](../.signpath/artifact-configuration.xml)
   names: the installers inside the artifact, with Authenticode.

Without those settings the workflow attaches the unsigned installers, as a
release built on a PC has today.

## Setting it up (the maintainer's steps)

SignPath's accounts and the Foundation's application are the maintainer's to
make; nobody else can make them for the project.

1. **Apply** at [signpath.org/apply](https://signpath.org/apply) for the
   open-source program, for `github.com/ifpintac/tequila`. What the
   application asks is answered in "The application" below.
2. When accepted, in SignPath:
   - make a **project** with the slug `tequila`;
   - link it to the GitHub repository (the "GitHub.com" trusted build
     system, with SignPath's GitHub app installed on the repository);
   - give it the artifact configuration in `.signpath/artifact-configuration.xml`;
   - make a signing policy with the slug `release-signing`, with the
     Foundation's certificate and manual approval.
3. In SignPath, make a **CI user** and copy its API token.
4. In GitHub, under **Settings > Secrets and variables > Actions**:
   - **Secrets**: `SIGNPATH_API_TOKEN`, the CI user's token;
   - **Variables**: `SIGNPATH_ORGANIZATION_ID`, the organization's id
     from SignPath. Set `SIGNPATH_PROJECT_SLUG` and
     `SIGNPATH_SIGNING_POLICY_SLUG` only if they differ from `tequila` and
     `release-signing`.
5. Turn on two-factor authentication for the GitHub account, which the
   Foundation requires of everyone who can commit.

The next run of the Windows release workflow then signs.

## The application

Answers for the Foundation's form, as they stand:

- **Project**: Tequila: Product Rendering
- **Repository**: https://github.com/ifpintac/tequila
- **Licence**: Apache-2.0 (OSI-approved)
- **What it is**: a free product-rendering application for Windows and
  macOS, for making studio pictures of 3D models: CAD import (STEP, IGES,
  glTF, OBJ, STL, USD, IFC), physically based materials, Cycles and its
  own path tracer, labels, environments, and scripting.
- **Releases**: published on GitHub since 2026-10-17, announced on the
  project's Discord.
- **What is to be signed**: the Windows installer and update installer,
  built by GitHub Actions from the repository
  (`.github/workflows/release-windows.yml`).
- **Code signing policy**: this page, linked from the README.
- **Maintainer**: ifpintac, the project's author.
