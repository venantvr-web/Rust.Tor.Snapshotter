# Consignes projet

## Langue

- Écrire en français avec les accents corrects (é, è, ê, à, â, î, ï, ô, ö,
  ù, û, ü, ç, œ, æ…). Jamais de français « sans accents ».
- Respecter la typographie française : espace insécable avant `;`, `:`,
  `!`, `?`, `»`, et après `«`. Guillemets français « … » pour les
  citations en texte courant ; les guillemets droits `"..."` restent dans
  le code et les valeurs techniques.
- Noms propres techniques (Rust, Tor, Google Drive, SQLite, CasaOS, axum,
  tokio, sqlx) : conserver la casse d'origine.

## Noms de la solution

Le projet a **quatre formes canoniques**, chacune liée à un usage précis. Ne
jamais les mélanger ; ne jamais inventer de variante (pas de « Rust Tor
Snapshotter » avec espaces, pas de « TorSnapshotter », etc.).

| Usage | Forme | Où |
| --- | --- | --- |
| Nom du dépôt et du projet (prose, titres, URLs, chemins `github.com/...`) | `Rust.Tor.Snapshotter` | Titre du README, URL du repo, icône CasaOS |
| Crate Rust et nom de l'exécutable | `rust_tor_snapshotter` | `Cargo.toml`, binaire installé, filtre `RUST_LOG`, `mod`/`use` éventuels |
| Image GHCR (contrainte de lowercase du registre) | `rust.tor.snapshotter` | tag `ghcr.io/venantvr-web/rust.tor.snapshotter` uniquement |
| Projet compose + conteneurs + chemins filesystem CasaOS | `rust-tor-snapshotter` | `name:` compose, `container_name:`, `/DATA/AppData/rust-tor-snapshotter`, `/var/lib/casaos/apps/rust-tor-snapshotter` |

- L'UI (titre HTML, `<h1>`, commentaires utilisateur) utilise
  `Rust.Tor.Snapshotter` — la forme lisible. Le snake_case
  `rust_tor_snapshotter` reste strictement technique.
- Un ancien archive `tor_snapshotter.tar.gz` a existé pendant la phase de
  reconstitution ; le dépôt est désormais directement exploitable, ne plus
  y faire référence.

## Tableaux

- Utiliser la **syntaxe Markdown pipe** (`| col | col |` avec séparateur
  `| --- | --- |`). C'est la forme attendue, même si le source ressemble
  à de l'ASCII : c'est GitHub qui rend le tableau proprement.
- Ne pas utiliser de blocs HTML `<table>` dans la doc Markdown.
- Les diagrammes (flux, architecture, séquence…) restent en Mermaid — la
  règle « pas d'ASCII art » s'applique aux dessins, pas aux tableaux
  structurés.

## Diagrammes

- **Toujours utiliser Mermaid**, jamais d'ASCII art, pour tout schéma
  d'architecture, flux, séquence, état, ER, etc.
- Intégrer via un bloc de code ` ```mermaid `. Exemple minimal :

  ```mermaid
  flowchart LR
    A[client] -- HTTPS --> B[app]
    B --> C[(SQLite)]
  ```

- Choisir le type de diagramme adapté :
  - `flowchart` pour l'architecture et les flux de données
  - `sequenceDiagram` pour les interactions temporelles client/serveur
  - `erDiagram` pour les schémas de base
  - `stateDiagram-v2` pour les machines à états
- Ne pas abuser des couleurs ni des styles — lisibilité avant tout.

## Application aux fichiers existants

Ces règles s'appliquent à tous les fichiers Markdown du dépôt
(`README.md`, docs futures, commentaires longs…). Si un diagramme ASCII
existe déjà, le convertir en Mermaid à la prochaine modification du
fichier.
