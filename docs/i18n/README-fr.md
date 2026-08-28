<h1 align="center">Apidoc (apidoc-rust)</h1>

<div align="center">
 Bibliothèque de plugins générique générant la documentation d'API via les proc-macros (proc-macro) de Rust
</div>

<div align="center">
<a href="https://github.com/erikwang2013/apidoc-rust"><img src="https://img.shields.io/badge/license-MIT-green"></a>
<a href="https://github.com/erikwang2013/apidoc-rust"><img src="https://img.shields.io/github/stars/erikwang2013/apidoc-rust"></a>
</div>

<div align="center">
<a href="../../README.md">中文</a> ·
<a href="README-en.md">English</a> ·
<a href="README-ko.md">한국어</a> ·
<a href="README-ru.md">Русский</a> ·
<a href="README-de.md">Deutsch</a> ·
<a href="README-fr.md"><strong>Français</strong></a> ·
<a href="README-es.md">Español</a> ·
<a href="README-pt.md">Português</a> ·
<a href="README-hi.md">हिन्दी</a> ·
<a href="README-ar.md">العربية</a> ·
<a href="README-bn.md">বাংলা</a> ·
<a href="README-id.md">Bahasa Indonesia</a> ·
<a href="README-ja.md">日本語</a>
</div>

## Présentation du projet

apidoc-rust est un **générateur de documentation d'API générique et modulaire** implémenté en Rust, inspiré de [apidoc-php](https://github.com/erikwang2013/apidoc-php) (une extension composer qui génère de la documentation d'API à partir des attributs PHP 8), transposant le principe « les annotations sont la documentation » de façon native en Rust :

- **Génération à la compilation** : la documentation est générée par les proc-macros à la compilation ; elle ne se désynchronise jamais du code ;
- **Collecte à coût nul** : enregistrement statique via linkme, une seule agrégation à l'exécution suffit pour obtenir toute la documentation des interfaces ;
- **Plugin générique** : le cœur est indépendant du framework HTTP et se connecte à n'importe quel framework via de minces adaptateurs (axum / actix-web).

## Fonctionnalités

### Implémenté (M1-M3)

- **Documentation par annotations** : sept macros d'attributs `title` / `desc` / `method` / `url` / `param` / `query` / `returned`, appliquées annotation par annotation (équivalent de la syntaxe des attributs PHP), avec paramètres imbriqués `required` / `default` / `desc` / `mock` / `children`
- **Validation à la compilation** : l'url doit commencer par `/`, méthode soumise à une liste blanche, `name` de paramètre obligatoire, etc. ; toute annotation invalide déclenche une erreur de compilation (span précis)
- **Collecte automatique** : enregistrement statique via linkme `distributed_slice`, sans liste manuelle d'interfaces ; `DocRegistry::collect()` fusionne par id et restaure l'ordre de déclaration par seq, avec collecte automatique inter-crates
- **Sortie api.json** : sérialisation serde du modèle de données unifié de la documentation (config + endpoints), champs alignés sur la sémantique PHP
- **Adaptateur axum + UI de documentation intégrée** : monter la route suffit pour obtenir la page de documentation, navigation par répertoires groupés (M2)
- **Complément d'annotations** : 12 nouvelles annotations `tag` / `group` / `author` / `header` / `route_param` / `response_status` / `success` / `error` / `not_debug` / `md` / `sort` / `ref` (M3)

### Implémenté (M4)

- **Débogage en ligne** : la page de documentation intègre un panneau « Débogage en ligne » — Base URL préremplie avec `location.origin` pour une connexion directe inter-domaines au service cible, préremplissage du formulaire de paramètres avec les valeurs Mock, remplacement des placeholders de route `{name}` / `:name`, paramètres GET/HEAD fusionnés dans la query string, corps JSON assemblé pour les autres méthodes, édition des en-têtes de requête + en-têtes personnalisés, affichage de la réponse (statut / temps / pretty JSON), avertissement jaune en cas d'échec CORS
- **Moteur Mock** (`crates/apidoc-mock`, dépend du crate fake, 15 règles : name / company / email / phone / url / ip / city / country / text / number / int / float / bool / uuid / date). Priorité des règles : `mock="fake:xxx"` suit la table des règles fake (nom inconnu → valeur par défaut) → les autres valeurs mock non vides sont renvoyées telles quelles (p. ex. `mock="1"`, `mock="erik"`) → sans mock, génération automatique selon `ty` (int→`"1"`, float→`"0.5"`, bool→`"true"`, object→`"{}"`, string→`"string"`) ; les children sont imbriqués récursivement, les array ont fixé 2 éléments
- **Interface mock** : l'adaptateur axum ajoute `GET /apidoc/mock?url=&method=`, correspondance exacte url + method, 404 si aucune correspondance ; le panneau de débogage masque par défaut les endpoints `not_debug`, qui n'apparaissent qu'en cochant « Afficher les interfaces not_debug »
- **Connexion CORS directe** : le débogage en ligne connecte le navigateur directement à l'interface cible, le `cors_layer` de l'adaptateur autorise la traversée (proxy inverse côté serveur réservé à v2)

### Prévu

- Multi-applications / multi-versions / mot de passe d'accès
- Export Markdown / TypeScript / Swagger (OpenAPI3) (M5)
- Adaptation multi-frameworks (apidoc-axum terminé, apidoc-actix à faire)
- v2 : générateur de code, référencement des champs de tables de données, liens de partage, événements de débogage

## Architecture

<img src="images/fr-architecture.svg" alt="Architecture générale d'apidoc-rust" width="100%">

## Fonctionnalités

<img src="images/fr-features.svg" alt="Fonctionnalités du projet apidoc-rust" width="100%">

## Cycle de vie

<img src="images/fr-lifecycle.svg" alt="Cycle de vie de la documentation apidoc-rust" width="100%">

## Structure du projet

```
apidoc-rust/
├── Cargo.toml                 # configuration du workspace (resolver 2)
├── VERSION                    # version du projet (v1.0.0, séparée de la version du framework 0.1.0)
├── crates/
│   ├── apidoc/                # cœur à l'exécution (indépendant du framework)
│   │   ├── src/lib.rs         # modèle de données + agrégation DocRegistry + api.json
│   │   ├── tests/             # tests d'intégration (expansion des macros/agrégation/sérialisation/inter-crates)
│   │   └── examples/demo.rs   # exemple : annotations + sortie api.json
│   ├── apidoc-macros/         # proc-macro : 19 macros d'attributs
│   │   └── src/lib.rs         # définitions des macros + analyse des paramètres + validation à la compilation
│   ├── apidoc-mock/           # moteur Mock (génération de données Mock par règles fake)
│   ├── apidoc-test-fixtures/  # fixtures de test d'enregistrement inter-crates
│   └── apidoc-axum/           # adaptateur axum (routes de documentation + cors_layer + /apidoc/mock)
├── .github/
│   └── workflows/release.yml  # workflow de publication (lit VERSION, crée tag + release de façon incrémentale)
└── docs/
    ├── images/                # schémas d'architecture/fonctionnalités/cycle de vie (SVG)
    └── i18n/                  # documentation multilingue (12 langues)
```

## Mode d'emploi

### 1. Ajouter les dépendances

```toml
[dependencies]
apidoc = "0.1"        # ou path = "crates/apidoc"
apidoc-macros = "0.1"
linkme = "0.3"        # l'expansion des macros référence directement le chemin linkme, le consommateur doit en dépendre directement
serde_json = "1"      # pour la sortie api.json
```

### 2. Écrire les annotations

En posant des annotations sur les fonctions handler, la documentation est générée à la compilation :

```rust
use apidoc::*;

#[apidoc::title("获取用户信息")]
#[apidoc::desc("根据用户 ID 查询用户详情")]
#[apidoc::url("/api/user/info")]
#[apidoc::method("GET")]
#[apidoc::param(name = "user_id", ty = "int", required, desc = "用户ID", mock = "1")]
#[apidoc::query(name = "lang", ty = "string", desc = "语言", default = "zh-CN")]
#[apidoc::returned(
    name = "data",
    ty = "object",
    desc = "用户数据",
    children = [
        { name = "id", ty = "int", required, desc = "用户ID" },
        { name = "name", ty = "string", required, desc = "用户名", mock = "erik" },
    ]
)]
fn get_user_info() -> String {
    unimplemented!()
}
```

### 3. Collecter et exporter

```rust
fn main() {
    let endpoints = DocRegistry::collect();
    let doc = ApiDoc {
        config: ApidocConfig {
            title: "我的 API".to_string(),
            description: None,
        },
        endpoints,
    };
    println!("{}", serde_json::to_string_pretty(&doc).unwrap());
}
```

### 4. Exécuter l'exemple

```bash
cargo run --example demo -p apidoc
```

Sortie (extrait) :

```json
{
  "config": { "title": "demo api" },
  "endpoints": [
    {
      "title": "获取用户信息",
      "desc": "根据用户 ID 查询用户详情",
      "url": "/api/user/info",
      "method": "GET",
      "params": [
        { "name": "user_id", "type": "int", "required": true, "desc": "用户ID", "mock": "1" }
      ],
      "querys": [
        { "name": "lang", "type": "string", "required": false, "default": "zh-CN", "desc": "语言" }
      ],
      "returned": [
        {
          "name": "data",
          "type": "object",
          "required": false,
          "desc": "用户数据",
          "children": [
            { "name": "id", "type": "int", "required": true, "desc": "用户ID" },
            { "name": "name", "type": "string", "required": true, "desc": "用户名", "mock": "erik" }
          ]
        }
      ]
    }
  ]
}
```

### 5. Débogage en ligne et Mock (M4)

Ouvrez la page de documentation → sélectionnez une interface → le panneau « Débogage en ligne » à droite préremplit les paramètres selon les règles Mock → pointez la Base URL vers l'adresse du service cible (par défaut `location.origin`, connexion directe inter-domaines) → cliquez sur Envoyer pour obtenir la réponse réelle (code de statut / temps / pretty JSON). Le panneau de débogage masque par défaut les endpoints `not_debug`, qui n'apparaissent qu'après avoir coché « Afficher les interfaces not_debug ».

**Exigence CORS** : le débogage en ligne connecte le navigateur directement à l'interface cible ; le service cible doit monter le `cors_layer` fourni par l'adaptateur pour autoriser les requêtes inter-domaines ; en cas d'échec CORS, le panneau affiche un avertissement jaune.

Syntaxe des règles Mock (trois niveaux de priorité) :

```rust
#[apidoc::param(name = "email", ty = "string", desc = "邮箱", mock = "fake:email")]  // généré par la règle fake
#[apidoc::param(name = "status", ty = "string", desc = "状态", mock = "1")]          // mock non vide renvoyé tel quel
#[apidoc::param(name = "name", ty = "string", desc = "用户名")]                       // sans mock : génération automatique selon ty
#[apidoc::returned(
    name = "data",
    ty = "object",
    children = [
        { name = "id", ty = "int", required },       // sans mock → "1"
        { name = "email", ty = "string", mock = "fake:email" },  // imbrication récursive des children
    ]
)]
fn create_user() -> String {
    unimplemented!()
}
```

15 règles fake intégrées : `name` / `company` / `email` / `phone` / `url` / `ip` / `city` / `country` / `text` / `number` / `int` / `float` / `bool` / `uuid` / `date` ; un nom de règle inconnu revient à la valeur par défaut. Génération automatique sans mock : int→`"1"`, float→`"0.5"`, bool→`"true"`, object→`"{}"`, string→`"string"` ; les array ont fixé 2 éléments.

## Feuille de route

| Phase | Contenu | Statut |
|------|------|------|
| M1 | squelette du workspace + modèle de données + MVP des macros + enregistrement linkme | ✅ Terminé |
| M2 | adaptateur axum + UI de documentation intégrée + répertoires groupés | ✅ Terminé |
| M3 | complément d'annotations (tag/group/author/header/route_param/response_status/success/error/not_debug/md/sort/ref) | ✅ Terminé |
| M4 | débogage en ligne + moteur Mock | ✅ Terminé |
| M5 | export markdown / typescript / swagger.json | Prévu |
| M6 | authentification par mot de passe, multi-applications multi-versions, publication | Prévu |

## Documentation multilingue

- [English](README-en.md)
- [한국어](README-ko.md)
- [Русский](README-ru.md)
- [Deutsch](README-de.md)
- [Français](README-fr.md)
- [Español](README-es.md)
- [Português](README-pt.md)
- [हिन्दी](README-hi.md)
- [العربية](README-ar.md)
- [বাংলা](README-bn.md)
- [Bahasa Indonesia](README-id.md)
- [日本語](README-ja.md)

## Soutien et dons

Si ce projet vous est utile, n'hésitez pas à nous soutenir avec une ⭐ Star, et les dons sont également les bienvenus pour l'open source !

### WeChat Pay (微信支付) / Alipay (支付宝)

<table>
  <tr>
    <td align="center">
      <img src="../../docs/weixinpay.png" width="130" height="130" alt="WeChat Pay (微信支付)" /><br/>
      <strong>微信支付 (WeChat Pay)</strong>
    </td>
    <td align="center">
      <img src="../../docs/alipay.png" width="130" height="130" alt="Alipay (支付宝)" /><br/>
      <strong>支付宝 (Alipay)</strong>
    </td>
  </tr>
</table>

### Dons par virement international

**【Informations du bénéficiaire】**

- Nom du bénéficiaire : WANG KEXUN
- Numéro de compte : 881015918251

**【Banque du bénéficiaire】**

- Code SWIFT de ZA Bank : AABLHKHHXXX
- Nom de la banque : ZA Bank Limited
- Code banque : 387
- Adresse de la banque : Core F, Cyberport 3, 100 Cyberport Road, Hong Kong

**【Banque correspondante pour virements transfrontaliers (si nécessaire)】**

> Veuillez noter qu'il s'agit de la banque correspondante (banque intermédiaire) pour les virements transfrontaliers, et non de la banque du bénéficiaire. Renseignez-vous auprès de votre banque émettrice pour savoir si ces informations sont requises.

- **La banque correspondante pour les virements en HKD, CNY et USD est Citibank :**
  - Nom de la banque : Citibank N.A. Hong Kong
  - Code SWIFT : CITIHKHXXXX
  - Code banque : 006
  - Nom de l'agence : Hong Kong Branch
  - Code d'agence : 391
  - Adresse de la banque : Citibank Tower, Citibank Plaza, 3 Garden Road, Central, Hong Kong
- **La banque correspondante pour les autres devises est BNY Mellon :**
  - Nom de la banque : THE BANK OF NEW YORK MELLON
  - Code SWIFT : IRVTUS3NXXX
  - Adresse de la banque : THE BANK OF NEW YORK MELLON, 240 GREENWICH STREET, NEW YORK, États-Unis

## License

[MIT](LICENSE)
