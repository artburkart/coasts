# Conductor

[Conductor](https://conductor.build/) は、並列の Claude Code エージェントを実行し、それぞれが独自の分離されたワークスペースを持ちます。ワークスペースは `~/conductor/workspaces/<project-name>/` に保存された git worktree です。各ワークスペースは名前付きブランチでチェックアウトされます。

これらの worktree はプロジェクトルートの外側に存在するため、Coast がそれらを検出してマウントするには明示的な設定が必要です。

## Setup

`~/conductor/workspaces/<project-name>` を `worktree_dir` に追加します。Codex（すべてのプロジェクトを 1 つのフラットなディレクトリ配下に保存する）とは異なり、Conductor は worktree をプロジェクトごとのサブディレクトリ配下にネストするため、パスにはプロジェクト名を含める必要があります。

```toml
[coast]
name = "my-app"
worktree_dir = [".worktrees", "~/conductor/workspaces/my-app"]
```

Conductor ではリポジトリごとにワークスペースパスを設定できるため、デフォルトの `~/conductor/workspaces` があなたの設定と一致しない場合があります。実際のパスを確認するには Conductor のリポジトリ設定を確認し、それに応じて調整してください — ディレクトリがどこにあっても原則は同じです。

Coast は実行時に `~` を展開し、`~/` または `/` で始まるパスを外部として扱います。詳細は [Worktree Directories](../coastfiles/WORKTREE_DIR.md) を参照してください。

`worktree_dir` を変更した後は、bind mount を有効にするために既存のインスタンスを**再作成**する必要があります。

```bash
coast rm my-instance
coast build
coast run my-instance
```

worktree の一覧はすぐに更新されます（Coast は新しい Coastfile を読み込みます）が、Conductor worktree への割り当てにはコンテナ内の bind mount が必要です。

## What Coast does

- **Bind mount** — コンテナ作成時に、Coast は `~/conductor/workspaces/<project-name>` をコンテナ内の `/host-external-wt/{index}` にマウントします。
- **Discovery** — `git worktree list --porcelain` はリポジトリスコープであるため、現在のプロジェクトに属する worktree のみが表示されます。
- **Naming** — Conductor の worktree は名前付きブランチを使用するため、Coast UI と CLI ではブランチ名で表示されます（例: `scroll-to-bottom-btn`）。1 つのブランチは同時に 1 つの Conductor ワークスペースでしかチェックアウトできません。
- **Assign** — `coast assign` は `/workspace` を外部 bind mount パスから再マウントします。
- **Gitignored sync** — ホストファイルシステム上で絶対パスを使って実行されるため、bind mount なしでも動作します。
- **Orphan detection** — git watcher は外部ディレクトリを再帰的にスキャンし、`.git` の gitdir ポインタでフィルタリングします。Conductor がワークスペースをアーカイブまたは削除した場合、Coast はインスタンスの割り当てを自動的に解除します。

## Example

```toml
[coast]
name = "my-app"
compose = "./docker-compose.yml"
worktree_dir = [".worktrees", ".claude/worktrees", "~/.codex/worktrees", "~/conductor/workspaces/my-app"]
primary_port = "web"

[ports]
web = 3000
api = 8080

[assign]
default = "none"
[assign.services]
web = "hot"
api = "hot"
```

- `.worktrees/` — Coast 管理の worktree
- `.claude/worktrees/` — Claude Code（ローカル、特別な処理なし）
- `~/.codex/worktrees/` — Codex（外部、bind-mounted）
- `~/conductor/workspaces/my-app/` — Conductor（外部、bind-mounted）

## Conductor Env Vars

- Coast 内のランタイム設定で Conductor 固有の環境変数（例: `CONDUCTOR_PORT`, `CONDUCTOR_WORKSPACE_PATH`）に依存するのは避けてください。Coast はポート、ワークスペースパス、サービスディスカバリを独立して管理します — 代わりに Coastfile の `[ports]` と `coast exec` を使用してください。
