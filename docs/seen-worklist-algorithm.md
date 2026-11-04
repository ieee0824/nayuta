# seen guard 付き worklist に対する条件付き計算量説明

## 要旨

Rust の構文木に対する既存のループ深さ解析は、共有の走査量を持つ探索でも入れ子の次数を合成する。本稿では、標準コレクションを用いる限定的な探索構文を認識し、既存候補に期待 O(V+E) の条件付き説明を付加するアルゴリズムを記述する。一般の償却解析や形式検証を実装するものではなく、全処理を列挙できる構文テンプレートに対象を限定する。

## 1. 問題設定

入力は `syn` により構築した Rust ソースファイルの構文木である。出力はソース位置と条件付きの計算量説明である。対象関数は固定の隣接リスト `edges: &[Vec<usize>]` と開始頂点 `start: usize` を受け取り、ローカルな worklist と訪問集合を生成する。[実装](../src/worklist.rs)および[再現例](../tests/fixtures/seen_worklist.rs)を参照されたい。

到達可能な頂点集合を U、V = |U|、E = Σ(v∈U) |edges[v]| と定義する。E は重複辺と自己ループを含む隣接リスト要素数である。グラフの構築コストは入力の外部で発生する。

## 2. 認識対象と仮定

認識する関数本体は次の形に限る。変数名は相異なる名前に変更できる。

```rust
let mut pending = vec![start];
let mut seen = HashSet::new();
while let Some(vertex) = pending.pop() {
    if !seen.insert(vertex) { continue; }
    for &next in &edges[vertex] { pending.push(next); }
}
```

関数全体の上限を説明するため、追加の文を許可しない。標準の Vec、HashSet、Some、vec! の識別を確認する。曖昧な名前、集合の変更、未知の呼び出し、異なるキーなどは認識失敗とする。頂点 ID の有効性、有限かつ固定のグラフ、usize の基本演算の定数コスト、ハッシュ操作の期待償却定数コスト、Vec の追加の償却定数コストを仮定する。

型検査・マクロ展開・cfg 評価は実施しない。トップレベル自由関数と同一ファイルの直接呼び出しに対象を限定し、非 doc 属性、glob import、トップレベルマクロなどがあるファイルは保守的に除外する。詳細な構文上の制約は[対応仕様](seen-worklist.md)に示す。

## 3. アルゴリズム

```text
Explain(file):
  scope ← CollectBindingsAndDirectImports(file)
  if attributes/macros/imports make scope opaque: return []
  functions ← uniquely bound top-level free functions
  walks ← empty map
  for f in functions:
    if MatchTraversal(f, scope):
      walks[f.name] ← position of its while statement
  notes ← explanations for entries of walks
  for f in functions not in walks:
    if MatchGroupCaller(f, scope, walks):
      append a caller explanation at its call position to notes
  return notes

MatchTraversal(f, scope):
  check exact signature and complete body shape
  resolve standard collection and constructor names conservatively
  check pop target, inserted key, adjacency index and pushed value
  check that all bindings are distinct and control flow matches
  accept only when every check succeeds
```

既存の構文候補は独立に計算する。認識成功時には前提付きの説明を追加し、既存の次数を引き下げない。失敗時には追加説明を生成しない。これは「実際に高次数である」という判断ではない。

## 4. 計算量説明の根拠

**補題1（隣接走査の一意性）.** 訪問集合は空から始まり、要素の追加以外で変更されない。同一頂点の insert が成功するのは一度だけである。したがって、各到達頂点の隣接リストは一度だけ走査され、内側反復回数と辺に由来する push 回数はそれぞれ E である。

**補題2（worklist の処理回数）.** 初期要素は一つであり、追加箇所は補題1の push だけである。成功する pop の総回数は 1+E である。有限の頂点・辺に対して追加回数が有限であるため、探索は終了する。終了を仮定して回数を制限する循環論法には依存しない。

**命題（テンプレートの条件付き上限）.** 第2節のコスト仮定の下で、認識された探索の期待実行時間は O(V+E) である。

*論証.* 成功する insert は V 回、試行は 1+E 回、隣接要素の処理は E 回である。他の文のコストは各反復当たり定数である。コレクションの期待・償却コストを適用すると総時間は O(1+V+E) となる。有効な開始頂点が存在するため V≥1 であり、O(V+E) と表記できる。この論証はテンプレートに対するものであり、任意の Rust プログラムや解析器実装の形式的健全性証明ではない。

## 5. 複数開始点の合成

対象の呼び出し元は、グループ列の各グループから開始頂点を取り出し、認識済み探索を一回ずつ呼ぶ完全な二重ループ本体に限る。G をグループ数、R を開始点・呼び出し総数、V_r と E_r を r 番目の呼び出しの到達部分グラフの量とする。

各呼び出しで seen が新規生成されるので、期待時間は O(G + Σ(r=1..R)(V_r+E_r)) である。空グループも G に寄与する。R=0 の場合も外側の走査は残る。最大値 V、E を用いれば O(G+R(V+E)) という粗い上限も得られる。共有訪問集合を渡す関数への一般化は行わない。厳密には関数呼び出しの固定コストを加えた O(1+G+Σ) であり、表示では固定項を省略する。

## 6. 解析器自体のコスト

構文木サイズを S、トップレベル関数数を F とする。属性走査、束縛収集、二回の関数走査を行う。名前表の期待定数時間アクセスと識別子・パスの通常の処理コストを前提とすると、追加パスの期待時間は O(S)、補助記憶は O(S) 以下である。テンプレート外の大きな本体は文数・構造検査で棄却する。これは追加パスのみの評価であり、既存の関数間解析、構文解析やファイル走査の総コストを表さない。

## 7. 検証

Rust のテストで正常例、標準項目の別名・完全修飾、既存次数候補との共存、呼び出し合成、各種の認識失敗を確認する。グラフ操作数について疎・密グラフ、循環、自己ループ、重複辺、非連結成分、空グループを扱う。CLI テストは説明位置、前提表示、--fail を確認する。実装時点で全38テスト、Clippy、整形チェックが成功した。

事前調査では1〜4頂点の全有向グラフと開始点の組263,714件について Python モデルの操作数も照合した。これは有限範囲の補助検証であり、Rust コードの実測性能や全入力に対する機械証明ではない。

反例として、guard 前の全頂点走査は Θ(V(1+E)) の追加仕事を生む。集合の初期化や guard 外からの無条件再追加は非停止を生み得る。このため insert の存在だけを根拠に次数を低減しない。

## 8. 関連研究と限界

Tarjan [1] は操作列全体の償却評価の基礎を与える。到達回数を区別する設計は [2]、増減とリセットの会計は [3]、関数要約の合成は [4] が参考となる。Rust の借用とリソース解析を統合する [5] は将来の型付き解析の参考である。これらが本稿の HashSet/Vec パターンをそのまま認識することを主張するものではない。

初期実装は固定構文への条件付き説明であり、一般のグラフ解析、共有状態の償却、trait dispatch、型・効果推論、完全な終了性検証を対象としない。

## 参考文献

1. R. E. Tarjan. *Amortized Computational Complexity*. SIAM Journal on Algebraic and Discrete Methods, 6(2), 306–318, 1985. [本文](https://www.math.u-szeged.hu/~hajnal/courses/MSc_AlgoBony/2021/Tarjan.pdf)
2. S. Gulwani and F. Zuleger. *The Reachability-Bound Problem*. PLDI, 2010. [本文](https://www.microsoft.com/en-us/research/wp-content/uploads/2016/12/pldi10_speed.pdf)
3. M. Sinn, F. Zuleger and H. Veith. *Complexity and Resource Bound Analysis of Imperative Programs Using Difference Constraints*. Journal of Automated Reasoning, 59, 3–45, 2017. [本文](https://pmc.ncbi.nlm.nih.gov/articles/PMC6044401/)
4. Q. Carbonneaux, J. Hoffmann and Z. Shao. *Compositional Certified Resource Bounds*. PLDI, 2015. [本文](https://www.cs.cmu.edu/~janh/assets/pdf/CarbonneauxHZ15.pdf)
5. Q. Lian and D. Wang. *Automatic Linear Resource Bound Analysis for Rust via Prophecy Potentials*. OOPSLA, 2025. [プレプリント](https://arxiv.org/html/2502.19810v1)
