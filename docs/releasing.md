# Publish a release

## Start a public repository without private history

A squash merge onto the existing branch keeps that branch's ancestors.
It does not remove earlier commits, author emails, deleted files, or old tags.

Keep one local repository. Create a parentless `public/main` branch from the
reviewed tree, then push only that branch to the public repository. Your private
development branches remain local and keep their full history.

```sh
public_tree=$(git rev-parse HEAD^{tree})
public_commit=$(printf '%s\n' 'Initial public release' | \
  git -c user.name=garyj \
      -c user.email=644451+garyj@users.noreply.github.com \
      commit-tree "$public_tree")
git branch public/main "$public_commit"
```

This creates one commit with the reviewed files and no parent. Verify that the
branch has one commit, no shared ancestor with the private branch, and the
expected public identity:

```sh
git rev-list --count public/main
git merge-base public/main fix/public-release || test $? -eq 1
git show --no-patch --format=fuller public/main
```

Create a blank public GitHub repository. Add its URL as `origin`, restrict the
default push ref, then publish `public/main` as GitHub's `main`:

```sh
git remote add origin git@github.com:garyj/yoink.git
git config remote.origin.push refs/heads/public/main:refs/heads/main
git push -u origin public/main:main
```

Do not use `git push --all`, `git push --mirror`, or push the private branches
or old tags. Do not merge a private branch into `public/main`. Apply future
changes to the public branch as ordinary commits or cherry-pick reviewed
changes without their private ancestors.

The single local `.git` directory still contains the private history. GitHub
receives only the refs you push. If you later clone the public repository, that
clone contains only the public history.

If sensitive information has already been published, a new snapshot does not
erase existing copies. See GitHub's
[history-removal guidance](https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/removing-sensitive-data-from-a-repository).

## Build and inspect the candidate

1. Set the same version in `package.json`, `src-tauri/tauri.conf.json`, and both
   Cargo manifests. Refresh `Cargo.lock` after changing Cargo package versions.
2. Install the pinned tools and dependencies as described in the README.
3. Run `just check` and the isolated X11 test from the README.
4. Commit the source changes. Run `just package` from the clean checkout.
5. Inspect the archives in `target/dist`. The binary archive contains `yoink`,
   its licence notices, and the source commit ID. The source archive contains
   the tracked source, vendored Cargo dependencies, and installed JavaScript
   dependency sources. It contains no Git history.
6. Extract the binary into a temporary directory. Run `./yoink --version` and
   exercise capture, search, copy, delete, pagination, and `--quit` in an
   isolated graphical session. Test the oldest Linux distribution you intend
   to support.

The package script builds through the Tauri CLI. A bare Cargo release build
does not embed the frontend. Staging directories remain inside `target/dist`
for inspection and are excluded from the uploaded artifacts.

The GitHub workflow builds on Ubuntu 22.04 to set an older glibc baseline than
the development workstation. The executable still needs GTK3, WebKitGTK 4.1,
an X11 display, and a graphical-session D-Bus connection. It is not a static
binary or an AppImage.

## Tag and publish

After the public repository is ready, tag the reviewed commit as `v0.1.0` and
push the tag. The CI workflow rejects a tag that differs from the manifest
versions. It creates a **draft** GitHub Release after the checks pass.

Inspect the draft's binary archive, source archive, and `SHA256SUMS`. Publish
the draft when the artifacts and release notes are ready. Preserve the source
archive alongside the binary so recipients can obtain the corresponding
source and dependency notices.

Test the actual published release through mise in a clean configuration:

```sh
mise use -g github:garyj/yoink@0.1.0
mise exec github:garyj/yoink@0.1.0 -- yoink --version
mise reshim
```

The expected version output is `yoink 0.1.0`. Quit any older yoink process
before starting the new executable. An upgrade does not replace an already
running process. If the repository is published under another owner, update
the README's `github:` examples.
