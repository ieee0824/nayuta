# nayuta

[English](README.en.md) | 日本語

Rustコードから、計算量やI/O回数が増えやすい箇所を見つける軽量な静的解析CLIです。

現在はループを入力サイズに依存するものと仮定し、次のパターンを検出します。

- 単一ループによる `O(N)` 候補
- 多重ループによる `O(N^m)` 候補
- 同期関数への多段呼び出し。走査対象内の `mod`、`crate::` / `self::` / `super::`、明示的な `use` 別名に対応
- `iter` / `iter_mut` / `into_iter` 起点のチェーンを消費する `collect`、`sum`、`count`、`for_each` など

`map`・`filter` の連結は一回の走査として扱い、消費時のクロージャ内のループや関数呼び出しを合成します。例えば線形処理の関数をループ内から呼ぶと `O(N^2)` 候補を報告します。

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
src/example.rs:12: complexity: loop may grow as O(N) (syntactic heuristic; assumes independent N-sized iterations; amortization and deduplication are not analyzed)
src/example.rs:18: complexity: loop nesting depth is 2; work may grow as O(N^2) (syntactic heuristic; assumes independent N-sized iterations; amortization and deduplication are not analyzed)
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
まだ区別しないため、検出結果はレビューの入り口として利用してください。型注釈付きの引数・ローカル変数と `self` について固有メソッドを解決します。`flat_map` / `flatten` は各内側の要素数も N に比例すると仮定し、一段ごとに次数を加えます。再帰・相互再帰は検出し、呼び出し元にもリスク候補を伝播します。例えば線形走査を伴う再帰は `potential O(N^2) or worse if recursion depth scales with N` と表示します。これは「再帰の深さも N に比例する場合、二乗以上の可能性」という条件付きのヒューリスティックです。走査のない再帰は `O(N)` 候補とし、一律に二乗とは扱いません。入力が半減する再帰などでは前提が成立しないため、厳密な上界・下界を意味しません。漸化式の解法や終了性の証明は行いません。

外部クレート、トレイト経由の呼び出し、型推論、glob import、多段の再エクスポート、`#[path]`、cfg の評価、関数ポインタ、変数に保存したイテレーターは未対応です。関数名のシャドーイングは考慮しません。解析対象に含まれないモジュールは読み込みません。同名の複数定義は解決せず、未解決の呼び出しのコストは合成しません。メモリ計測はLinuxのperfを利用します。

診断は各反復が独立に N に比例するという構文上の推定です。共有の走査量、seen guard による重複除去、償却計算量、入力サイズ間の関係は解析しません。例えば seen guard 付きグラフ探索は、ハッシュ操作の期待定数時間を仮定すれば O(V+E) でも、二重ループとして O(N^2) 候補を報告します。
