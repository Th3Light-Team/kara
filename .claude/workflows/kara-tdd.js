export const meta = {
  name: 'kara-tdd',
  description: 'Opus fija el contrato y escribe las pruebas; Sonnet implementa sin poder tocarlas; Opus supervisa contra la spec.',
  whenToUse: 'Para construir una conveniencia de ground/spec/ con separacion de poderes: quien implementa no escribe su propio criterio de aceptacion. Pasa en args la conveniencia y el crate objetivo.',
  phases: [
    { title: 'Contrato', detail: 'Opus lee la conveniencia y fija API publica, casos borde y criterios', model: 'opus' },
    { title: 'Pruebas', detail: 'Opus escribe los tests y los stubs todo!(); deben fallar de partida', model: 'opus' },
    { title: 'Implementacion', detail: 'Sonnet rellena la implementacion hasta que pasen', model: 'sonnet' },
    { title: 'Auditoria', detail: 'cargo test + clippy + sha256 de los ficheros de prueba' },
    { title: 'Supervision', detail: 'Opus revisa contra la conveniencia y las reglas del proyecto', model: 'opus' },
  ],
}

// ---------------------------------------------------------------------------
// Entrada
// ---------------------------------------------------------------------------
const task = typeof args === 'string' ? { conveniencia: args } : (args || {})

if (!task.conveniencia) {
  throw new Error(
    'Falta args.conveniencia. Ejemplo:\n' +
    '{ conveniencia: "Eliminar a papelera", spec: "ground/spec/05-operaciones.md", crate: "kara-fs" }'
  )
}

const CONVENIENCIA = task.conveniencia
const SPEC_FILE = task.spec || 'ground/spec/'
const CRATE = task.crate || 'kara-core'
const MAX_ATTEMPTS = task.maxAttempts || 3

// Reglas del proyecto que TODO agente debe respetar. Copiadas de CLAUDE.md:
// si CLAUDE.md cambia, esto se actualiza a mano — es deliberado, para que el
// contrato del workflow no se mueva sin que alguien lo decida.
const RULES = `
REGLAS DEL PROYECTO (CLAUDE.md) — no negociables:
- ground/spec/ es la fuente de verdad. No inventes comportamiento.
- Prohibido unwrap() / expect() en cualquier ruta que toque ficheros del usuario.
- Cada syscall devuelve Result; el error se propaga o se reporta, nunca se traga.
- Eliminar va siempre a la papelera FreeDesktop. Lo irreversible confirma.
- Mover, copiar, renombrar y crear entran en la pila de deshacer.
- Un fallo en un lote no aborta el lote: reintentar / omitir / cancelar.
- Capas: ui -> ops -> {fs, index} -> core. Nunca al reves.
- QML no contiene logica de negocio.
- Codigo, identificadores y comentarios en INGLES. Documentacion en espanol.
- Nada debe bloquear la UI, ni con 100k entradas ni con un volumen colgado.
`

// ---------------------------------------------------------------------------
// Esquemas
// ---------------------------------------------------------------------------
const CONTRACT = {
  type: 'object',
  required: ['api', 'edgeCases', 'acceptance', 'files'],
  properties: {
    summary: { type: 'string', description: 'Que hace esta conveniencia, en 2 frases' },
    api: {
      type: 'array',
      description: 'Firmas publicas Rust exactas que expondra el crate',
      items: {
        type: 'object',
        required: ['signature', 'purpose'],
        properties: {
          signature: { type: 'string' },
          purpose: { type: 'string' },
          errors: { type: 'string', description: 'Que variantes de error puede devolver y cuando' },
        },
      },
    },
    edgeCases: {
      type: 'array',
      description: 'Casos borde extraidos de la spec, cada uno verificable',
      items: {
        type: 'object',
        required: ['case', 'expected'],
        properties: {
          case: { type: 'string' },
          expected: { type: 'string' },
          fromSpec: { type: 'string', description: 'Cita textual breve de la spec que lo respalda' },
        },
      },
    },
    acceptance: {
      type: 'array',
      description: 'Criterios binarios de aceptacion',
      items: { type: 'string' },
    },
    files: {
      type: 'array',
      description: 'Ficheros que se crearan o tocaran',
      items: { type: 'string' },
    },
  },
}

const TESTS_WRITTEN = {
  type: 'object',
  required: ['testFiles', 'stubFiles', 'baselineFails'],
  properties: {
    testFiles: {
      type: 'array',
      description: 'Ficheros de prueba escritos, con su sha256 recien calculado',
      items: {
        type: 'object',
        required: ['path', 'sha256', 'testCount'],
        properties: {
          path: { type: 'string' },
          sha256: { type: 'string' },
          testCount: { type: 'integer' },
        },
      },
    },
    stubFiles: {
      type: 'array',
      description: 'Ficheros de implementacion con firmas + todo!() que Sonnet debe rellenar',
      items: { type: 'string' },
    },
    baselineFails: {
      type: 'boolean',
      description: 'true si cargo test falla AHORA (si pasa, las pruebas no prueban nada)',
    },
    baselineOutput: { type: 'string' },
    coverageOfEdgeCases: {
      type: 'array',
      description: 'Cada caso borde del contrato mapeado al nombre del test que lo cubre',
      items: {
        type: 'object',
        required: ['case', 'testName'],
        properties: { case: { type: 'string' }, testName: { type: 'string' } },
      },
    },
  },
}

const AUDIT = {
  type: 'object',
  required: ['testsPass', 'testsIntact', 'clippyClean', 'noStubsLeft'],
  properties: {
    testsPass: { type: 'boolean' },
    testOutput: { type: 'string', description: 'Ultimas ~40 lineas de cargo test' },
    testsIntact: { type: 'boolean', description: 'false si algun sha256 de fichero de prueba cambio' },
    tamperDetail: { type: 'string', description: 'Que fichero de prueba cambio y como' },
    clippyClean: { type: 'boolean' },
    clippyOutput: { type: 'string' },
    noStubsLeft: { type: 'boolean', description: 'false si queda todo!() o unimplemented!()' },
    ignoredTests: { type: 'integer', description: 'Numero de #[ignore] encontrados en los tests' },
    riskyUnwraps: {
      type: 'array',
      description: 'unwrap()/expect() fuera de bloques #[cfg(test)]',
      items: { type: 'string' },
    },
  },
}

const REVIEW = {
  type: 'object',
  required: ['approved', 'findings'],
  properties: {
    approved: { type: 'boolean' },
    rationale: { type: 'string' },
    findings: {
      type: 'array',
      items: {
        type: 'object',
        required: ['severity', 'file', 'issue', 'fix'],
        properties: {
          severity: { type: 'string', enum: ['bloqueante', 'importante', 'menor'] },
          file: { type: 'string' },
          line: { type: 'integer' },
          issue: { type: 'string' },
          fix: { type: 'string' },
          rule: { type: 'string', description: 'Que regla de CLAUDE.md o que caso borde de la spec incumple' },
        },
      },
    },
    specGaps: {
      type: 'array',
      description: 'Comportamiento de la spec que ni el contrato ni las pruebas cubrieron',
      items: { type: 'string' },
    },
  },
}

// ---------------------------------------------------------------------------
// Fase 1 — Contrato (Opus)
// ---------------------------------------------------------------------------
phase('Contrato')
log(`Conveniencia: ${CONVENIENCIA} -> crate ${CRATE}`)

const contract = await agent(`
Eres el arquitecto de Kara, un explorador de ficheros en Rust + QML.
Trabajas en el repositorio /home/oliverv/Projects/kara.

TAREA: fijar el contrato de la conveniencia "${CONVENIENCIA}".

1. Lee CLAUDE.md.
2. Lee ${SPEC_FILE} y localiza la conveniencia "${CONVENIENCIA}". Lee su
   behavior COMPLETO, incluidos todos los casos borde. Si no la encuentras con
   ese nombre exacto, busca por grep en ground/spec/ y usa la mas cercana,
   diciendo cual has usado.
3. Lee el estado actual del crate ${CRATE} para no duplicar ni romper nada.

Produce el contrato: la API publica exacta (firmas Rust reales, con tipos de
error concretos), los casos borde VERIFICABLES sacados de la spec (cada uno con
su cita), y criterios de aceptacion binarios.

No escribas codigo todavia. No implementes. Solo el contrato.
Se exhaustivo con los casos borde: es lo unico que impedira que la
implementacion sea plausible-pero-incorrecta.

${RULES}
`, { model: 'opus', schema: CONTRACT, label: `contrato:${CONVENIENCIA}` })

log(`Contrato: ${contract.api.length} firmas, ${contract.edgeCases.length} casos borde`)

// ---------------------------------------------------------------------------
// Fase 2 — Pruebas y stubs (Opus)
// ---------------------------------------------------------------------------
phase('Pruebas')

const tests = await agent(`
Eres el supervisor de calidad de Kara. Trabajas en /home/oliverv/Projects/kara.

Este es el contrato acordado para "${CONVENIENCIA}":
${JSON.stringify(contract, null, 2)}

TAREA — en este orden:

1. Escribe los STUBS: las firmas publicas del contrato en el crate ${CRATE},
   con cuerpo todo!("..."). Tipos, structs y variantes de error SI van
   implementados. Debe compilar.

2. Escribe las PRUEBAS. Una por caso borde del contrato, con nombre que diga
   que verifica. Cubre TODOS los casos borde. Para pruebas que tocan el disco,
   usa directorios temporales y limpialos.

3. Ejecuta 'cargo test -p ${CRATE}' y comprueba que FALLA (por los todo!()).
   Si pasara, las pruebas no prueban nada: reescribelas.

4. Calcula el sha256 de cada fichero de prueba con sha256sum y devuelvelo.

REGLA CRITICA: las pruebas son el criterio de aceptacion objetivo. Otro modelo
implementara contra ellas SIN poder modificarlas. Escribelas para que solo
pasen con una implementacion correcta, no con una que las contente. Nada de
assert!(true), nada de tests tautologicos.

${RULES}
`, { model: 'opus', schema: TESTS_WRITTEN, label: `pruebas:${CRATE}` })

if (!tests.baselineFails) {
  log('AVISO: las pruebas pasan sin implementacion. El criterio no vale — abortando.')
  return { aborted: 'baseline_passes', contract, tests }
}

const totalTests = tests.testFiles.reduce((n, f) => n + f.testCount, 0)
log(`${totalTests} pruebas escritas en ${tests.testFiles.length} fichero(s); baseline falla, correcto`)

// ---------------------------------------------------------------------------
// Fases 3-5 — Implementar / Auditar / Supervisar, en bucle
// ---------------------------------------------------------------------------
const hashList = tests.testFiles.map(f => `${f.path}  ${f.sha256}`).join('\n')
const history = []
let approved = false
let lastAudit = null
let lastReview = null

for (let attempt = 1; attempt <= MAX_ATTEMPTS; attempt++) {
  const feedback = history.length
    ? `\nINTENTOS ANTERIORES (corrige esto):\n${JSON.stringify(history, null, 2)}\n`
    : ''

  phase('Implementacion')
  await agent(`
Eres el implementador de Kara. Trabajas en /home/oliverv/Projects/kara.
Intento ${attempt} de ${MAX_ATTEMPTS}.

Contrato:
${JSON.stringify(contract, null, 2)}

Ficheros a rellenar: ${tests.stubFiles.join(', ')}
Ficheros de prueba (INTOCABLES): ${tests.testFiles.map(f => f.path).join(', ')}
${feedback}
TAREA: sustituye cada todo!() por la implementacion real hasta que
'cargo test -p ${CRATE}' pase entero y 'cargo clippy --all-targets' salga limpio.

PROHIBIDO:
- Modificar CUALQUIER fichero de prueba. Se verifica por sha256 y si cambian,
  el intento se descarta entero.
- Anadir #[ignore] a un test.
- Debilitar un assert, o cambiar la firma publica del contrato.
- Dejar todo!() o unimplemented!().
- unwrap()/expect() fuera de codigo de prueba.

Si crees que una prueba esta MAL, no la toques: dilo en tu respuesta final
explicando por que, y deja el resto implementado.

${RULES}
`, { model: 'sonnet', label: `implementa:intento-${attempt}` })

  phase('Auditoria')
  lastAudit = await agent(`
Auditor mecanico. Trabajas en /home/oliverv/Projects/kara. NO opines, NO
arregles nada, NO edites ficheros. Solo ejecuta y reporta.

1. cargo test -p ${CRATE} 2>&1 | tail -40
2. cargo clippy --all-targets 2>&1 | grep -E '^(error|warning)' | head -30
3. sha256sum de cada fichero de prueba, y COMPARA con estos valores esperados:
${hashList}
4. grep -rn 'todo!\\|unimplemented!' en crates/${CRATE}/src
5. grep -rn '#\\[ignore\\]' en los ficheros de prueba
6. grep -rn 'unwrap()\\|expect(' en crates/${CRATE}/src, excluyendo bloques de test

Reporta los hechos tal cual.
`, { model: 'haiku', effort: 'low', schema: AUDIT, label: `auditoria:intento-${attempt}` })

  if (!lastAudit.testsIntact) {
    log(`Intento ${attempt}: PRUEBAS ALTERADAS — ${lastAudit.tamperDetail}`)
    history.push({ attempt, rechazado: 'ficheros de prueba modificados', detalle: lastAudit.tamperDetail })
    continue
  }
  if (!lastAudit.testsPass || !lastAudit.noStubsLeft) {
    log(`Intento ${attempt}: tests en rojo o quedan stubs`)
    history.push({ attempt, rechazado: 'tests fallan', salida: lastAudit.testOutput })
    continue
  }

  phase('Supervision')
  lastReview = await agent(`
Eres el supervisor de Kara. La implementacion pasa las pruebas — eso es
condicion necesaria, no suficiente. Tu trabajo es buscar lo que las pruebas NO
capturan. Se adversarial: por defecto, desaprueba.

Conveniencia: "${CONVENIENCIA}"  ·  Spec: ${SPEC_FILE}  ·  Crate: ${CRATE}
Contrato:
${JSON.stringify(contract, null, 2)}

Revisa el codigo implementado en crates/${CRATE} y comprueba:
1. Cada caso borde del contrato: leelo en la spec y verifica en el CODIGO que se
   cumple, no solo que hay un test verde.
2. Las reglas de CLAUDE.md, una por una.
3. Comportamiento de la spec que ni el contrato ni las pruebas cubrieron
   (specGaps) — es el fallo mas caro y el que nadie mas va a ver.
4. Que la implementacion no sea un caso especial cosido a los tests.

${RULES}
`, { model: 'opus', schema: REVIEW, label: `supervision:intento-${attempt}` })

  const blockers = lastReview.findings.filter(f => f.severity === 'bloqueante')
  if (lastReview.approved && blockers.length === 0) {
    approved = true
    log(`Intento ${attempt}: APROBADO`)
    break
  }

  log(`Intento ${attempt}: rechazado por supervision — ${blockers.length} bloqueante(s)`)
  history.push({ attempt, rechazado: 'supervision', hallazgos: lastReview.findings })
}

return {
  conveniencia: CONVENIENCIA,
  crate: CRATE,
  aprobado: approved,
  intentos: history.length + (approved ? 1 : 0),
  contrato: contract,
  pruebas: { total: totalTests, ficheros: tests.testFiles.map(f => f.path) },
  auditoriaFinal: lastAudit,
  supervisionFinal: lastReview,
  historial: history,
}
