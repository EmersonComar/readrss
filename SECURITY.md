# Segurança

## Versões suportadas

Apenas a versão mais recente publicada em Releases recebe correções de segurança.

## Como relatar uma vulnerabilidade

Não abra uma issue pública. Use o relato privado do GitHub: aba **Security** do repositório →
**Report a vulnerability**. Inclua a versão afetada, os passos para reproduzir e o impacto esperado.

Você receberá uma resposta assim que possível; correções são publicadas numa nova versão e
anunciadas nas notas da release.

## Verificando um download

Cada release traz o arquivo `SHA256SUMS` e um atestado de procedência gerado pelo GitHub Actions,
que comprova que o binário foi compilado por este repositório a partir da tag correspondente:

```sh
sha256sum -c SHA256SUMS
gh attestation verify readrss-<versão>-linux-x86_64.tar.gz --repo EmersonComar/readrss
gh attestation verify readrss_<versão>-1_amd64.deb --repo EmersonComar/readrss
```

Pelo repositório APT, o `apt` confere sozinho a assinatura GPG do repositório (chave
`1C99A7EB636500B38B66EC037998AF819108FCAA`), que nunca sai do servidor de publicação.

## O que o CI verifica

- `cargo deny`: vulnerabilidades conhecidas (base RustSec), licenças compatíveis com a GPL-3.0,
  dependências duplicadas e origem das dependências (somente crates.io). Roda também semanalmente.
- `cargo clippy` com avisos tratados como erro, formatação e os testes automatizados.
- Ações do GitHub fixadas por hash de commit e atualizadas pelo Dependabot.
