# Mermaid組み込みの検証

本文は図の生成を待たずに表示します。`/`で「検証本文」を検索できます。

## フローチャート

```mermaid
flowchart TD
    A[文書を開く] --> B{図があるか}
    B -->|はい| C[図を生成する]
    B -->|いいえ| D[本文を読む]
    C --> D
```

検証本文その1。生成中も検索やスクロールを試してください。

## クラス図

```mermaid
classDiagram
    class Document {
        +String title
        +open()
    }
    class Diagram {
        +String source
        +render()
    }
    Document "1" --> "*" Diagram : 図を含む
```

検証本文その2。ウィンドウ幅を変えて図の大きさを確認してください。

## ER図

```mermaid
erDiagram
    DOCUMENT ||--o{ DIAGRAM : "図を含む"
    DOCUMENT {
        int id PK "文書ID"
        string title "見出し"
    }
    DIAGRAM {
        int id PK "図ID"
        int document_id FK "文書ID"
        string source "図の定義"
    }
```

検証本文その3。図の後にも本文を表示します。

通常のコードブロックは引き続きコードとして表示します。

```rust
fn main() {
    println!("Mermaidではないコード");
}
```
