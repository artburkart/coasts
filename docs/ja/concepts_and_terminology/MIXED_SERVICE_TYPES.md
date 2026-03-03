# 混在サービス種別

ほとんどのプロジェクトは1つのモデルにきれいに当てはまります。すべてをオーケストレーションする `docker-compose.yml` か、プレーンなプロセスを実行する `[services]` のどちらかです。中には両方を同時に必要とするプロジェクトもあります。Coast は同じ Coastfile 内で `compose` と `[services]` の両方を定義できるようにしており、Docker Compose によって重量級でステートフルなサービスを管理しつつ、DinD ホスト上に置くべき軽量ツールは素のプロセスで扱えます。

## 混在サービスが必要なとき

もっとも一般的なシナリオは大規模なモノレポで、コアとなるアプリケーションスタック（Web サーバー、データベース、バックグラウンドワーカー）はすでに Docker Compose でコンテナ化されている一方、いくつかの開発ツールはホストレベルのプロセスとして動かしたほうが適しているケースです。

- `host.docker.internal` 経由で compose コンテナの内側から到達できる必要がある **Vite / Webpack dev サーバー**。DinD ホスト上で bare サービスとして動かすことでネットワーク周りのハックを避けられます。
- 内側のコンテナの overlay ファイルシステムを経由せず、バインドマウントされた `/workspace` へ高速にファイルシステムアクセスする必要がある **ファイルウォッチャーやコードジェネレーター**。
- コンテナイメージに焼き込むよりも、プロセスとして実行したほうが簡単な **言語サーバーやビルドツール**。

compose サービスがホストレベルのプロセスへホスト名で到達する必要があるなら、混在サービスが正しい答えです。

## 設定

Docker Compose スタックには `compose` を設定し、同じ Coastfile 内に bare プロセス用の `[services.*]` セクションを追加します。

```toml
[coast]
name = "my-app"
compose = "./docker-compose.yml"

[coast.setup]
packages = ["nodejs", "npm"]

# Bare services run directly on the DinD host
[services.vite-web]
install = "cd /workspace && npm install"
command = "cd /workspace && npm run dev"
port = 3040
restart = "on-failure"

# Compose services are defined in docker-compose.yml as usual
# (web, postgres, redis, sidekiq, etc.)

[ports]
web = 3000
vite-web = 3040
postgres = 5432

[assign]
default = "none"

[assign.services]
web = "restart"
sidekiq = "restart"
vite-web = "restart"
```

`[coast.setup]` セクションは bare サービスに必要なパッケージをインストールします。compose サービスは従来どおり Dockerfile からランタイムを取得します。

## 仕組み

`coast run` では、Coast が両方のサービス種別を順に起動します。

1. まず compose サービスを DinD コンテナ内で `docker compose up -d` によって起動し、ヘルスチェックが通るまで待機します
2. 次に bare サービスを `/coast-supervisor/` 内の supervisor スクリプトにより起動します

起動後は両方が並行して動作します。compose サービスは DinD デーモンにより管理される内側のコンテナとして実行されます。bare サービスは DinD ホスト OS 上のプレーンなプロセスとして実行されます。

```text
┌─── Coast: dev-1 ──────────────────────────────────────────┐
│                                                            │
│   Inner Docker daemon (compose services)                   │
│   ├── web           (Rails, :3000)                         │
│   ├── postgres      (database, :5432)                      │
│   └── redis         (cache, :6379)                         │
│                                                            │
│   /coast-supervisor/ (bare services)                       │
│   ├── vite-web.sh   (node process, :3040)                  │
│   └── vite-admin.sh (node process, :3041)                  │
│                                                            │
│   /workspace ← bind mount of project root                  │
└────────────────────────────────────────────────────────────┘
```

## サービス種別間のネットワーキング

compose サービスは内側の Docker デーモンのネットワーク内で動作します。bare サービスは DinD ホスト上で動作します。compose コンテナから bare サービスへ到達できるようにするには:

- bare サービスは `0.0.0.0` にバインドして、Docker ネットワーク内から到達できるようにします
- compose コンテナは `host.docker.internal` 経由で bare サービスへ到達できます（DinD コンテナ内で自動的に利用可能です）
- compose ファイルで bare サービスを指す環境変数を設定します: `VITE_HOST=host.docker.internal:3040`

bare サービスは、compose のポートが DinD ホスト上に公開されているため、`localhost:<published-port>` 経由で compose コンテナへ到達できます。

## コマンド

すべての Coast コマンドは、両方のサービス種別に対して透過的に動作します。

| Command | Compose services | Bare services |
|---|---|---|
| `coast ps` | `kind: compose` として表示 | `kind: bare` として表示 |
| `coast logs` | `docker compose logs` 経由で取得 | `/var/log/coast-services/` から tail |
| `coast logs --service <name>` | compose サービスなら compose にルーティング | bare サービスなら bare にルーティング |
| `coast restart-services` | compose down + up を実行 | stop-all + start-all を実行 |
| `coast assign` | compose の rebuild/restart を処理 | install コマンドを再実行して再起動 |
| `coast stop` / `coast start` | compose を停止/開始 | bare サービスを停止/開始 |

特定サービスのログを要求すると、Coast がサービス種別を自動判定し、適切なログソースへルーティングします。

## ブランチ切り替え

`coast assign` では、両方のサービス種別が処理されます。

1. compose サービスは assign 戦略（`restart`、`rebuild`、`hot`）に応じて停止するか、強制再作成されます
2. bare サービスは停止され、install コマンドが再実行され、サービスが再起動します

`[assign.services]` セクションは compose サービス名と bare サービス名の両方を受け付けます。どちらにも同じ戦略を使ってください。

```toml
[assign.services]
web = "rebuild"          # compose service — rebuild image on branch switch
sidekiq = "restart"      # compose service — restart container
vite-web = "restart"     # bare service — stop, re-install, restart
```

## 共有サービス

[共有サービス](SHARED_SERVICES.md) は compose と bare のどちらのサービスとも独立して動作します。ホストの Docker デーモン上で実行され、どのサービス種別が設定されているかに関わらず Coast 内からアクセスできます。

## bare のみから混在への移行

bare サービスのみの Coastfile から始めて、一部のサービスをコンテナ化しつつ他は bare プロセスのまま維持したい場合:

1. コンテナ化したいサービス向けに Dockerfile と `docker-compose.yml` を作成します
2. `[coast]` セクションに `compose = "./docker-compose.yml"` を追加します
3. すでに compose に入ったサービスについては `[services.*]` セクションを削除します
4. bare のままにしたいプロセスについては `[services.*]` を維持します
5. `coast build` で再ビルドします
