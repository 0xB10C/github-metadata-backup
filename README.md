# github-metadata-backup

Download issues and pull-requests from the GitHub API and store as JSON files.
Supports incremental updates. The tool is written in Rust. Please

## Usage

The GitHub API is rate-limted to 60 requests per hour without authentication. This
tool requires a [GitHub Personal Access Token], which increases the rate-limit to
5000 requests per hour. A _classic_ token is recommended over the newer _fine-grained
tokens_. Some of the events might be missing from the backup with the _fine-grained
tokens_. See https://github.com/orgs/community/discussions/58534 for more information.

[GitHub Personal Access Token]: (https://github.com/settings/tokens)

The token an either be supplied to the `github-metadata-backup` binary via the
`-p`/`--personal-access-token` command line option or placed in a file and read
from there with the `-f`/`--personal-access-token-file` option. The tool also
requires an `--owner` (a GitHub user or organization) and the `--repo` (the
repository of that owner) to download the metadata from. The backup will be
placed in the directory defined with `-d`/`--destination`. Make sure the tool
has the required permissions to write to it.

```
Usage: github-metadata-backup [OPTIONS] --owner <OWNER> --repo <REPO> --destination <PATH>

Options:
  -o, --owner <OWNER>
          Owner of the repository to backup
  -r, --repo <REPO>
          Name of the repository to backup
  -p, --personal-access-token <PERSONAL_ACCESS_TOKEN>
          Personal Access Token to the GitHub API supplied via the command line
  -f, --personal-access-token-file <PATH>
          Personal Access Token to the GitHub API read from a file
  -d, --destination <PATH>
          Destination where the backup should be written to
  -h, --help
          Print help
  -V, --version
          Print version
```

The log-level can be controlled with the `RUST_LOG` environment variable. By
default, it's `RUST_LOG=info`.

## Example

To backup the metadata from the `bitcoin/bitcoin` repository to the `bitcoin-bitcoin`
directory while reading the file from the `read-only-github-access-token.sec` file the
following command can be used:

```
github-metadata-backup --owner bitcoin --repo bitcoin --destination bitcoin-bitcoin --personal-access-token-file read-only-github-access-token.sec
```

This creates the `bitcoin-bitcoin` directory with the `issues` and `pulls`
subdirectories. It requests metadata until the rate-limit is reached, waits
until requests are allowed again, and then continues. Once finished with the
initial backup, which can take a few hours with large repositories, it writes
a `state.json` file. On subsequent runs, this file is read and only an
incremental backup is made. To do a full backup again, delete the state.json
file. The old backup will be overwritten. The JSON files are formatted to be
easily trackable in git. It makes sense to commit each incremental backup.

## Nix Package and module

A Nix package and module for the github-metadata-backup tool are avaliable in
my personal [nix-package-collection]. The package can be run with:

```
nix run github:0xb10c/nix#github-metadata-backup
```

[nix-package-collection]: https://github.com/0xB10C/nix

