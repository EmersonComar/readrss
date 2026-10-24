# readrss

Leitor de feeds RSS/Atom para desktop Linux, em Rust com GTK4 + libadwaita.

Inspirado no [NewsFlash](https://gitlab.com/news-flash/news_flash_gtk). Desenvolvido com o auxílio de
inteligência artificial para atender a algumas particularidades do autor.

## Funcionalidades

- Feeds RSS/Atom adicionados pela URL do feed **ou do site** (o feed é descoberto automaticamente).
- Três modos de leitura por artigo:
  - **Resumo**: o conteúdo vindo do feed.
  - **Completo**: o texto principal é extraído da página (dom_smoothie, port do Readability) e exibido no leitor; fica em cache.
  - **Site**: a página original dentro do app (WebKitGTK).
- Ícones robustos: cascata `<link rel=icon>`/`apple-touch-icon` → ícone do feed → `/favicon.ico`,
  validados e normalizados para PNG; sem ícone válido, avatar com as iniciais.
- Sincronização automática a cada 5 min, 30 min ou 1 h, com ETag/Last-Modified.
- Notificação de novos artigos quando a janela não está em foco; continua rodando em segundo plano ao fechar a janela.
- Não lidos, favoritos, busca, marcar tudo como lido, importação/exportação OPML.
- Interface em português, inglês e espanhol (Preferências → Idioma; troca sem reiniciar).
- Menu principal com **Changelog** e **Sobre o readrss** (com as novidades da versão).
- Clique direito num feed: **Editar** (endereço e nome personalizado), **Filtrar**, marcar como lido, remover.
- **Filtros por feed**: palavras-chave ou regex (uma por linha), para ocultar as notícias que casam ou mostrar
  só elas. Sem diferenciar maiúsculas/acentos; palavras simples casam só a palavra inteira. As notícias
  filtradas ficam ocultas (não apagadas), não contam como não lidas nem notificam; favoritos nunca somem.

## Instalação

Há duas formas: baixar o binário pronto ou compilar a partir do código-fonte. As duas instalam o
mesmo layout e ativam o serviço de segundo plano:

| Onde | O quê |
|---|---|
| `/opt/readrss` | binário (~5 MB) + traduções e ícones; usa GTK/WebKit do sistema |
| `/usr/local/bin/readrss` | atalho para rodar `readrss` no terminal |
| `/usr/local/share/applications`, `/usr/local/share/icons` | lançador e ícones |
| `/etc/systemd/user/readrss.service` | serviço **de usuário**, ativado para quem instalou |

### Binário pronto (Ubuntu 26.04 ou equivalente, x86_64)

O binário usa as bibliotecas do sistema e exige versões recentes (GTK 4.18+, libadwaita 1.7+,
WebKitGTK 2.50+). Em distribuições mais antigas, compile a partir do código-fonte.

```sh
sudo apt install libgtk-4-1 libadwaita-1-0 libwebkitgtk-6.0-4 libsqlite3-0 gstreamer1.0-plugins-bad
```

Baixe `readrss-<versão>-linux-x86_64.tar.gz` e `SHA256SUMS` da [página de Releases](https://github.com/EmersonComar/readrss/releases), verifique e instale:

```sh
sha256sum -c SHA256SUMS
gh attestation verify readrss-<versão>-linux-x86_64.tar.gz --repo EmersonComar/readrss   # opcional
tar xzf readrss-<versão>-linux-x86_64.tar.gz
cd readrss-<versão>-linux-x86_64
./install.sh                 # pede a senha do sudo apenas para copiar os arquivos
```

O `gh attestation verify` confirma que o arquivo foi compilado pelo GitHub Actions deste repositório
a partir da tag da versão.

### Compilando a partir do código-fonte

```sh
sudo apt install rustc cargo libgtk-4-dev libadwaita-1-dev libwebkitgtk-6.0-dev libsqlite3-dev libssl-dev \
                 gettext gstreamer1.0-plugins-bad
./scripts/install.sh         # compila em modo release e instala
```

### Desinstalar

```sh
./install.sh --uninstall           # a partir do pacote baixado
./scripts/install.sh --uninstall   # a partir do código-fonte
```

Seus feeds e artigos continuam em `~/.local/share/readrss`.

O `gstreamer1.0-plugins-bad` é usado pelo WebKit para vídeos (H.264, legendas WebVTT). Sem ele os vídeos
das páginas podem não tocar e o terminal mostra avisos como `GStreamer element h264parse not found`.

### Serviço em segundo plano

O serviço sobe com a sessão gráfica (`graphical-session.target`) rodando `readrss --background`:
sincroniza e notifica sem janela aberta. Abrir o readrss pelo menu (ou clicar numa notificação)
apenas mostra a janela dessa instância. “Sair” no menu encerra tudo até o próximo login.

```sh
systemctl --user status readrss      # estado
journalctl --user -u readrss         # logs
```

## Desenvolvimento

```sh
cargo run                                    # usa o banco real e o ID do app instalado
READRSS_APP_ID=io.github.readrss.Dev \
READRSS_DATA_DIR=/tmp/readrss-dev cargo run  # isolado, ao lado da versão instalada
```

Com o serviço instalado rodando e o ID padrão, `cargo run` apenas abre a janela da versão instalada.
`READRSS_APP_ID`, `READRSS_DATA_DIR` e `READRSS_CACHE_DIR` permitem rodar isolado (os testes de
interface usam isso). Traduções e ícones vêm de `data/` no repositório e de `<prefixo>/share/readrss/`
na instalação.

`Ctrl+C` no terminal encerra o app normalmente (ele também responde a SIGTERM).

## Atalhos

| Tecla | Ação |
|---|---|
| `j` / `n`, `k` / `p` | Próximo / anterior artigo |
| `f` | Alternar resumo ↔ artigo completo |
| `s` | Favoritar |
| `m` | Alternar lido / não lido |
| `o` | Abrir no navegador |
| `Ctrl+N` | Adicionar feed |
| `Ctrl+R` / `F5` | Sincronizar agora |
| `Ctrl+F` | Buscar |
| `Ctrl+,` | Preferências |
| `Ctrl+Q` | Sair |

Clique direito num feed para editar, filtrar, marcar como lido ou remover.

## Testes

```sh
cargo test                     # tudo; os testes de interface abrem janelas (sem display, são pulados)
cargo test --lib --test net --test sync --test i18n --test reader --test icons   # sem abrir janelas, ~1 s
cargo test --test gui          # só a interface
cargo test -p extract-strings  # o extrator de traduções
```

A rede é simulada com um servidor HTTP local (`FakeWeb` em `tests/common/mod.rs`), então os testes não
dependem de nenhum site no ar. `tests/i18n.rs` varre o código e falha se algum texto em
`gettext()`/`ngettext()`/`N_()` estiver sem tradução ou com placeholder diferente em algum idioma.
Os testes de interface também rodam o binário compilado para verificar o modo `--background` e o
encerramento por SIGINT/SIGTERM durante uma sincronização.

## Traduções

Os textos do código ficam em inglês dentro de `gettext()`/`ngettext()`/`ngettext_n()`/`N_()`.
Os catálogos estão em `po/` e os `.mo` compilados em `data/locale/` (versionados, pois o build não
compila traduções). O `xgettext` não suporta Rust; o extrator fica em `tools/extract-strings`.

Depois de alterar textos no código ou um `.po`:

```sh
./scripts/update-translations.sh   # extrai para po/readrss.pot, atualiza os .po e compila os .mo
```

Novo idioma: `msginit -i po/readrss.pot -l <código> -o po/<código>.po --no-translator`, depois
adicione o código em `po/LINGUAS` e em `LANGUAGES` (`src/i18n.rs`).

## Lançando uma versão

1. Atualize `version` no `Cargo.toml`.
2. Adicione um `Release` no topo de `RELEASES` em `src/changelog.rs` (entradas em inglês dentro de `N_()`).
3. Rode `./scripts/update-translations.sh`, traduza as novas entradas em `po/*.po` e rode o script de novo.
4. `cargo test` — há testes que falham se o changelog não tiver a versão nova ou se faltar tradução.
5. Crie e envie a tag: `git tag v<versão> && git push origin v<versão>`.

O workflow `release.yml` roda as mesmas checagens do CI, confere se a tag bate com o `Cargo.toml`,
compila num contêiner Ubuntu 26.04, gera o `.tar.gz`, o `SHA256SUMS` e o atestado de procedência, e
publica a release.

## Integração contínua e segurança

- `ci.yml` (push, pull request e semanalmente): `cargo deny` (vulnerabilidades da base RustSec,
  licenças compatíveis com a GPL-3.0, duplicatas e origem das dependências), `cargo fmt`,
  `cargo clippy -D warnings` e os testes de unidade e integração, num contêiner Ubuntu 26.04.
- Ações do GitHub fixadas por hash de commit; o Dependabot atualiza ações e dependências.
- HTTPS valida certificados pela lista de autoridades do sistema (atualizada pelo Ubuntu).
- Relato de vulnerabilidades: veja [SECURITY.md](SECURITY.md).

## Ícone

`data/icons/hicolor/` segue a estrutura do tema hicolor, com o ID do app como nome:

- `scalable/apps/io.github.readrss.ReadRSS.svg` — ícone principal (grade de 128px do GNOME HIG)
- `symbolic/apps/io.github.readrss.ReadRSS-symbolic.svg` — versão monocromática de 16px

O app registra essa pasta no tema de ícones ao iniciar (funciona sem instalar); o
`install.sh` copia os arquivos para `/usr/local/share/icons`. Para o snap, aponte
`icon:` do `snapcraft.yaml` para o SVG principal e use `Icon=io.github.readrss.ReadRSS` no `.desktop`.

## Dados

- Banco: `~/.local/share/readrss/readrss.db` (SQLite). Mudanças de esquema são migrações versionadas
  (`MIGRATIONS` em `src/db.rs`, controladas por `PRAGMA user_version`): acrescente, nunca edite as publicadas.
- Cache de ícones: `~/.cache/readrss/icons/`

## Estrutura

```
src/
  main.rs          ponto de entrada
  app.rs           Adw.Application: ações, timer de sync, notificações, OPML, preferências, --background
  window.rs        janela: fontes | lista | leitor
  html.rs          HTML/CSS do leitor (links absolutos, sem URL base)
  sync.rs          sincronização: rede em threads, gravação no banco na thread da interface
  net.rs           HTTP, parsing de feeds, descoberta, ícones, extração
  db.rs            SQLite (com migrações)
  filters.rs       regras de filtro (palavras/regex)
  feed_dialogs.rs  diálogos Editar e Filtrar do menu de contexto
  opml.rs          import/export OPML
  i18n.rs          gettext e troca de idioma em tempo de execução
  about.rs         diálogos Sobre e Changelog
  changelog.rs     histórico de versões
  constants.rs     ID do app e localização de traduções/ícones
  metrics.rs       memória e processos WebKit (usado nos testes)
tests/             net, sync, i18n, reader, icons e gui (testes de unidade ficam nos próprios módulos)
data/              locale/ (traduções compiladas) e icons/
po/                catálogos de tradução (pt_BR, es)
tools/             extract-strings: extrator de textos traduzíveis
scripts/           install.sh, package-release.sh, update-translations.sh, ci-install-deps.sh
.github/           workflows de CI e release, Dependabot
```

## Licença

GPL-3.0-or-later. Veja [LICENSE](LICENSE).
