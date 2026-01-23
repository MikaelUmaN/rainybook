# rainybook
Order book and market microstructure

## Development

### Git worktrees
This repository is intended to be worked on using `git worktree`. Either clone `--bare` as `rainybook.git` or develop only inside `worktrees/`, never in the root clone directly.

The `.cargo/config.toml` uses `target-dir = "../target"`, so all worktrees share a single build cache at `worktrees/target/`. This keeps the build artifacts inside the repo structure.

```
rainybook/                  # root clone (don't work here)
  worktrees/
    target/                 # shared build cache
    main/                   # main branch worktree
    feature-X/
    feature-Y/
```

#### Initial setup

```bash
git clone git@github.com:MikaelUmaN/rainybook.git
cd rainybook
mkdir worktrees
git worktree add worktrees/main main
```

#### Useful commands

```bash
# List all worktrees
git worktree list

# Prune stale worktree metadata
git worktree prune
```

#### Feature development

```bash
git worktree add worktrees/feature-xyz -b feature-xyz origin/main
# Work...
# Finish feature
git push -u origin feature-xyz
git worktree remove worktrees/feature-xyz
```

#### Review/Contribute to existing branch

```bash
git fetch --all --prune
git worktree add worktrees/feature-abc origin/feature-abc
# Review or contribute
# Push changes if you made any
git push
git worktree remove worktrees/feature-abc
```
