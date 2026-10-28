# nayuta

Rustコードから、計算量やI/O回数が増えやすい箇所を見つける軽量な静的解析CLIです。

現在はループを入力サイズに依存するものと仮定し、次のパターンを検出します。

- 単一ループによる `O(N)` 候補
- 多重ループによる `O(N^m)` 候補

## 使い方

```console
cargo run -- path/to/project
```

引数なしではカレントディレクトリを再帰的に調べます。`target` と `.git` は除外されます。

```console
# 検出があったら終了コード1を返す（CI向け）
cargo run -- --fail src
```

## 出力例

```text
src/example.rs:12: complexity: loop may grow as O(N)
src/example.rs:18: complexity: loop nesting depth is 2; work may grow as O(N^2)
```

## Linuxでのメモリアクセス計測

Linuxのperfと、メモリサンプリングに対応するCPU・カーネルが必要です。ソース行を表示するには、
対象をデバッグ情報付きでビルドしてください（例: Cargo.tomlの `[profile.release]` に `debug = true`）。

```console
cargo build --release
nayuta memory --output run.data -- ./target/release/example argument
nayuta memory --input run.data
```

`perf mem record` で対象を実行し、`perf mem report` でソース行・シンボル別のサンプル数と
分布を表示します。保存済みデータの再表示でもLinuxとperfが必要です。既存の出力ファイルは拒否します。
失敗時にはperfの診断を表示し、記録データを調査用に残します。権限不足の場合はホストのperf設定を確認してください。

値は正確な読み書き回数ではなくサンプルです。CPUによって対応イベントやサンプリングの偏りが異なり、
高レイテンシのロードが優先される場合があります。割合はキャッシュミス率ではありません。
アクセス先領域の独自集計、キャッシュミス率の算出、入力サイズに対する増加率の推定は未対応です。
詳細は [perf mem](https://manpages.debian.org/trixie/linux-perf/perf-mem.1.en.html) を参照してください。

## 制限

現在の検査はRustの構文木に基づくヒューリスティックです。固定回数のループと入力サイズに依存するループを
まだ区別しないため、検出結果はレビューの入り口として利用してください。イテレーターチェーン、再帰、
呼び出し先をまたぐ計算量は未実装です。メモリ計測はLinuxのperfを利用します。
