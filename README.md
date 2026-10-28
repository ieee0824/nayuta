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

## 制限

現在の検査はRustの構文木に基づくヒューリスティックです。固定回数のループと入力サイズに依存するループを
まだ区別しないため、検出結果はレビューの入り口として利用してください。イテレーターチェーン、再帰、
呼び出し先をまたぐ計算量、およびOSのデバッグAPIを用いた動的解析は未実装です。
