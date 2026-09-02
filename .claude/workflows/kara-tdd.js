export const meta = {
  name: 'kara-tdd',
  description: 'Opus fija el contrato y escribe las pruebas; Sonnet implementa sin poder tocarlas; Opus supervisa. Con checkpoints en disco: reanudable entre sesiones.',
  whenToUse: 'Para construir una conveniencia de ground/spec/ con separacion de poderes. Reanudable: si una corrida muere a medias, la siguiente retoma desde el ultimo checkpoint en .kara/progress/.',
  phases: [
    { title: 'Checkpoint', detail: 'Lee .kara/progress/ y decide desde donde retomar' },
    { title: 'Contrato', detail: 'Opus fija API, casos borde y criterios; los persiste', model: 'opus' },
    { title: 'Pruebas', detail: 'Opus escribe tests + stubs, los commitea y anota sus sha256', model: 'opus' },
    { title: 'Implementacion', detail: 'Sonnet rellena la implementacion sin tocar los tests', model: 'sonnet' },
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
    '{ conveniencia: "Enviar a la papelera", spec: "ground/spec/05-operaciones.md", crate: "kara-fs" }'
  )
}

const CONVENIENCIA = task.conveniencia
const SPEC_FILE = task.spec || 'ground/spec/'
const CRATE = task.crate || 'kara-core'
const MAX_ATTEMPTS = task.maxAttempts || 3
const REPO = '/home/oliverv/Projects/kara'

// Slug determinista (sin Date/Math.random: romperian el resume del runtime).
const SLUG = CONVENIENCIA
  .normalize('NFD').replace(/[̀-ͯ]/g, '')
  .toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '').slice(0, 60)

const LEDGER = `.kara/progress/${SLUG}.json`

// Reglas copiadas de CLAUDE.md a proposito: el contrato del workflow no debe
// moverse solo porque alguien edite CLAUDE.md. Sincronizar a mano.
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

const LEDGER_SPEC = `
El fichero de checkpoint es ${REPO}/${LEDGER} (JSON). Estructura:
{
  "conveniencia": "...", "crate": "...", "slug": "${SLUG}",
  "estado": "contrato_listo" | "pruebas_listas" | "aprobada" | "fallida",
  "contrato": { ...contrato completo... },
  "pruebas": { "testFiles": [{"path","sha256","testCount"}], "stubFiles": [...] },
  "intentos": 0,
  "historial": [...],
  "actualizado": "<salida de: date -Is>"
}
Crea el directorio con mkdir -p si hace falta. Escribe SIEMPRE el fichero entero
(no lo edites por partes) y no borres campos que ya estuvieran puestos.
`

// ---------------------------------------------------------------------------
// Esquemas
// ---------------------------------------------------------------------------
const CHECKPOINT = {
  type: 'object',
  required: ['existe', 'estado'],
  properties: {
    existe: { type: 'boolean' },
    estado: {
      type: 'string',
      enum: ['ninguno', 'contrato_listo', 'pruebas_listas', 'aprobada', 'fallida'],
    },
    intentosPrevios: { type: 'integer' },
    testFiles: {
      type: 'array',
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
    stubFiles: { type: 'array', items: { type: 'string' } },
    resumen: { type: 'string', description: 'Que habia hecho ya, en una linea' },
  },
}

const CONTRACT_DONE = {
  type: 'object',
  required: ['ok', 'apiCount', 'edgeCaseCount'],
  properties: {
    ok: { type: 'boolean' },
    apiCount: { type: 'integer' },
    edgeCaseCount: { type: 'integer' },
    summary: { type: 'string' },
    conveniencasAbsorbidas: {
      type: 'array',
      description: 'Otras conveniencias de la spec que resultan inseparables de esta',
      items: { type: 'string' },
    },
  },
}

const TESTS_WRITTEN = {
  type: 'object',
  required: ['testFiles', 'stubFiles', 'baselineFails', 'committed'],
  properties: {
    testFiles: {
      type: 'array',
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
    stubFiles: { type: 'array', items: { type: 'string' } },
    baselineFails: { type: 'boolean' },
    baselineOutput: { type: 'string' },
    committed: { type: 'boolean', description: 'true si el commit de contrato+pruebas salio bien' },
    commitSha: { type: 'string' },
  },
}

const AUDIT = {
  type: 'object',
  required: ['testsPass', 'testsIntact', 'clippyClean', 'noStubsLeft'],
  properties: {
    testsPass: { type: 'boolean' },
    testOutput: { type: 'string' },
    testsIntact: { type: 'boolean' },
    tamperDetail: { type: 'string' },
    clippyClean: { type: 'boolean' },
    clippyOutput: { type: 'string' },
    noStubsLeft: { type: 'boolean' },
    ignoredTests: { type: 'integer' },
    riskyUnwraps: { type: 'array', items: { type: 'string' } },
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
          rule: { type: 'string' },
        },
      },
    },
    specGaps: { type: 'array', items: { type: 'string' } },
  },
}

// ---------------------------------------------------------------------------
// Salida parcial: cualquier muerte por presupuesto devuelve esto, nunca revienta
// ---------------------------------------------------------------------------
function interrumpido(fase, extra) {
  log(`INTERRUMPIDO en fase "${fase}". El checkpoint queda en ${LEDGER}.`)
  return {
    conveniencia: CONVENIENCIA,
    crate: CRATE,
    slug: SLUG,
    aprobado: false,
    interrumpido: true,
    faseAlcanzada: fase,
    checkpoint: LEDGER,
    reanudar: `Vuelve a lanzar kara-tdd con los mismos args; retomara desde el checkpoint.`,
    ...(extra || {}),
  }
}

// ---------------------------------------------------------------------------
// Fase 0 — Checkpoint
// ---------------------------------------------------------------------------
phase('Checkpoint')

const ck = await agent(`
Trabajas en ${REPO}. Tarea puramente mecanica, no edites nada.

Mira si existe el fichero ${LEDGER}. Si existe, leelo y reporta su estado.
Si no existe, reporta estado "ninguno".

${LEDGER_SPEC}
`, { model: 'haiku', effort: 'low', schema: CHECKPOINT, label: `checkpoint:${SLUG}` })

const estadoPrevio = ck?.estado || 'ninguno'

if (estadoPrevio === 'aprobada') {
  log(`Ya estaba aprobada en una corrida anterior. Nada que hacer.`)
  return { conveniencia: CONVENIENCIA, crate: CRATE, slug: SLUG, aprobado: true, yaEstaba: true }
}

if (ck?.existe) log(`Retomando desde checkpoint: ${estadoPrevio} — ${ck.resumen || ''}`)

// ---------------------------------------------------------------------------
// Fase 1 — Contrato (Opus) — se salta si ya estaba
// ---------------------------------------------------------------------------
const contratoYaEsta = ['contrato_listo', 'pruebas_listas'].includes(estadoPrevio)

if (!contratoYaEsta) {
  phase('Contrato')
  log(`Conveniencia: ${CONVENIENCIA} -> crate ${CRATE}`)

  const contract = await agent(`
Eres el arquitecto de Kara, un explorador de ficheros en Rust + QML.
Trabajas en ${REPO}.

TAREA: fijar el contrato de la conveniencia "${CONVENIENCIA}".

1. Lee CLAUDE.md.
2. Lee ${SPEC_FILE} y localiza "${CONVENIENCIA}". Lee su behavior COMPLETO,
   incluidos todos los casos borde. Si no la encuentras con ese titulo exacto,
   busca por grep en ground/spec/ y usa la mas cercana, diciendo cual.
3. Si hay conveniencias hermanas inseparables de esta (p.ej. sentido de
   ordenacion respecto a ordenar por), absorbelas en el mismo contrato y
   listalas: implementar medio comportamiento obliga a rehacerlo despues.
4. Lee el estado actual del crate ${CRATE} para no duplicar ni romper nada.

ESCRIBE el contrato completo en ${LEDGER} con estado "contrato_listo":
la API publica exacta (firmas Rust con tipos de error concretos), los casos
borde VERIFICABLES sacados de la spec (cada uno con su cita textual breve), y
criterios de aceptacion binarios.

No implementes nada. Solo el contrato. Se exhaustivo con los casos borde: es lo
unico que impedira que la implementacion sea plausible-pero-incorrecta.

${LEDGER_SPEC}
${RULES}
`, { model: 'opus', schema: CONTRACT_DONE, label: `contrato:${SLUG}` })

  if (!contract?.ok) return interrumpido('Contrato')
  log(`Contrato: ${contract.apiCount} firmas, ${contract.edgeCaseCount} casos borde`)
  if (contract.conveniencasAbsorbidas?.length) {
    log(`Absorbe tambien: ${contract.conveniencasAbsorbidas.join(' · ')}`)
  }
}

// ---------------------------------------------------------------------------
// Fase 2 — Pruebas y stubs (Opus) — se salta si ya estaban
// ---------------------------------------------------------------------------
let testFiles = ck?.testFiles || []
let stubFiles = ck?.stubFiles || []

if (estadoPrevio !== 'pruebas_listas') {
  phase('Pruebas')

  const tests = await agent(`
Eres el supervisor de calidad de Kara. Trabajas en ${REPO}.

El contrato acordado para "${CONVENIENCIA}" esta en ${LEDGER}. LEELO PRIMERO.

TAREA — en este orden:

1. Escribe los STUBS: las firmas publicas del contrato en el crate ${CRATE},
   con cuerpo todo!("..."). Tipos, structs y variantes de error SI van
   implementados. Debe compilar.

2. Escribe las PRUEBAS. Una por caso borde del contrato, con nombre que diga
   que verifica. Cubre TODOS los casos borde. Para pruebas que tocan el disco,
   usa directorios temporales y limpialos.

3. Ejecuta 'cargo test -p ${CRATE}' y comprueba que FALLA (por los todo!()).
   Si pasara, las pruebas no prueban nada: reescribelas.

4. Calcula el sha256 de cada fichero de prueba con sha256sum.

5. Actualiza ${LEDGER} a estado "pruebas_listas" anadiendo testFiles y stubFiles.

6. COMMIT: 'git add -A && git commit' con mensaje
   "test(${CRATE}): contrato y pruebas de ${CONVENIENCIA}" y el trailer
   "Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>".
   Este commit es el seguro: si la corrida muere durante la implementacion, el
   trabajo caro (contrato + pruebas) ya esta a salvo en git.

REGLA CRITICA: las pruebas son el criterio de aceptacion objetivo. Otro modelo
implementara contra ellas SIN poder modificarlas. Escribelas para que solo pasen
con una implementacion correcta, no con una que las contente. Nada de
assert!(true), nada de tests tautologicos.

${LEDGER_SPEC}
${RULES}
`, { model: 'opus', schema: TESTS_WRITTEN, label: `pruebas:${CRATE}` })

  if (!tests) return interrumpido('Pruebas')

  if (!tests.baselineFails) {
    log('ABORTADO: las pruebas pasan sin implementacion, el criterio no vale.')
    return { conveniencia: CONVENIENCIA, crate: CRATE, slug: SLUG, aprobado: false, abortado: 'baseline_passes' }
  }

  testFiles = tests.testFiles || []
  stubFiles = tests.stubFiles || []
  if (!tests.committed) log('AVISO: el commit de contrato+pruebas no se confirmo.')
}

if (!testFiles.length) return interrumpido('Pruebas', { motivo: 'sin ficheros de prueba en el checkpoint' })

const totalTests = testFiles.reduce((n, f) => n + (f.testCount || 0), 0)
const hashList = testFiles.map(f => `${f.path}  ${f.sha256}`).join('\n')
log(`${totalTests} pruebas en ${testFiles.length} fichero(s)`)

// ---------------------------------------------------------------------------
// Fases 3-5 — Implementar / Auditar / Supervisar, en bucle
// ---------------------------------------------------------------------------
const history = []
let approved = false
let lastAudit = null
let lastReview = null

for (let attempt = 1; attempt <= MAX_ATTEMPTS; attempt++) {
  const feedback = history.length
    ? `\nINTENTOS ANTERIORES (corrige esto):\n${JSON.stringify(history, null, 2)}\n`
    : ''

  phase('Implementacion')
  const impl = await agent(`
Eres el implementador de Kara. Trabajas en ${REPO}.
Intento ${attempt} de ${MAX_ATTEMPTS}.

El contrato esta en ${LEDGER}. LEELO PRIMERO.

Ficheros a rellenar: ${stubFiles.join(', ')}
Ficheros de prueba (INTOCABLES): ${testFiles.map(f => f.path).join(', ')}
${feedback}
TAREA: sustituye cada todo!() por la implementacion real hasta que
'cargo test -p ${CRATE}' pase entero y 'cargo clippy --all-targets' salga limpio.

PROHIBIDO:
- Modificar CUALQUIER fichero de prueba. Se verifica por sha256 y si cambian,
  el intento se descarta entero. Tampoco los reformatees con cargo fmt.
- Anadir #[ignore] a un test.
- Debilitar un assert, o cambiar la firma publica del contrato.
- Dejar todo!() o unimplemented!().
- unwrap()/expect() fuera de codigo de prueba.

Si crees que una prueba esta MAL, no la toques: dilo en tu respuesta final
explicando por que, y deja el resto implementado.

Trabaja de forma incremental y guarda a menudo: si te quedas sin margen a mitad,
lo que hayas escrito en disco se aprovecha en la siguiente corrida.

${RULES}
`, { model: 'sonnet', label: `implementa:intento-${attempt}` })

  if (impl === null) return interrumpido('Implementacion', { intento: attempt, historial: history })

  phase('Auditoria')
  lastAudit = await agent(`
Auditor mecanico. Trabajas en ${REPO}. NO opines, NO arregles nada, NO edites
ficheros. Solo ejecuta y reporta.

1. cargo test -p ${CRATE} 2>&1 | tail -40
2. cargo clippy --all-targets 2>&1 | grep -E '^(error|warning)' | head -30
3. sha256sum de cada fichero de prueba, y COMPARA con estos valores esperados:
${hashList}
4. grep -rn 'todo!\\|unimplemented!' en crates/${CRATE}/src
5. grep -rn '#\\[ignore\\]' en los ficheros de prueba
6. grep -rn 'unwrap()\\|expect(' en crates/${CRATE}/src, excluyendo bloques de test

Reporta los hechos tal cual.
`, { model: 'haiku', effort: 'low', schema: AUDIT, label: `auditoria:intento-${attempt}` })

  if (!lastAudit) return interrumpido('Auditoria', { intento: attempt, historial: history })

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

  // Punto de retorno por intento: la auditoria ya paso (tests verdes, clippy
  // limpio, pruebas intactas). Commitear AQUI permite volver al mejor intento si
  // uno posterior empeora el codigo, y salva el trabajo si el limite salta
  // durante la supervision.
  await agent(`
Trabajas en ${REPO}. Tarea mecanica. La auditoria del intento ${attempt} paso.
Haz 'git add -A && git commit' con mensaje:
"wip(${CRATE}): intento ${attempt} de ${CONVENIENCIA} (auditoria verde, sin supervisar)"
y el trailer "Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>".
Devuelve el sha corto. No hagas nada mas.
`, { model: 'haiku', effort: 'low', label: `wip:intento-${attempt}` })

  phase('Supervision')
  lastReview = await agent(`
Eres el supervisor de Kara. La implementacion pasa las pruebas — condicion
necesaria, no suficiente. Busca lo que las pruebas NO capturan. Se adversarial:
por defecto, desaprueba.

Conveniencia: "${CONVENIENCIA}"  ·  Spec: ${SPEC_FILE}  ·  Crate: ${CRATE}
El contrato esta en ${LEDGER}. LEELO PRIMERO.

Revisa el codigo implementado en crates/${CRATE} y comprueba:
1. Cada caso borde del contrato: leelo en la spec y verifica en el CODIGO que se
   cumple, no solo que hay un test verde.
2. Las reglas de CLAUDE.md, una por una.
3. Comportamiento de la spec que ni el contrato ni las pruebas cubrieron
   (specGaps) — es el fallo mas caro y el que nadie mas va a ver.
4. Que la implementacion no sea un caso especial cosido a los tests.

${RULES}
`, { model: 'opus', schema: REVIEW, label: `supervision:intento-${attempt}` })

  if (!lastReview) return interrumpido('Supervision', { intento: attempt, historial: history })

  const blockers = lastReview.findings.filter(f => f.severity === 'bloqueante')

  // La puerta es OBJETIVA: cero bloqueantes. El campo `approved` del supervisor
  // es solo advisory — se le instruye ser adversarial y desaprobar por defecto,
  // asi que casi nunca dice true. Exigir ambos rechazaba intentos limpios y
  // dejaba que el siguiente intento empeorase el codigo (ocurrio de verdad).
  if (blockers.length === 0) {
    approved = true
    log(`Intento ${attempt}: APROBADO (0 bloqueantes; supervisor advisory: ${lastReview.approved})`)
    if (lastReview.findings.length) {
      log(`  quedan ${lastReview.findings.length} hallazgo(s) no bloqueantes, anotados para revision humana`)
    }
    break
  }

  log(`Intento ${attempt}: rechazado — ${blockers.length} bloqueante(s)`)
  history.push({ attempt, rechazado: 'supervision', bloqueantes: blockers.length, hallazgos: lastReview.findings })
}

// ---------------------------------------------------------------------------
// Cierre: sella el checkpoint
// ---------------------------------------------------------------------------
phase('Checkpoint')
await agent(`
Trabajas en ${REPO}. Cierra el checkpoint de "${CONVENIENCIA}".

Actualiza ${LEDGER} poniendo estado "${approved ? 'aprobada' : 'fallida'}",
intentos ${history.length + (approved ? 1 : 0)}, el historial de rechazos, y
"actualizado" con la salida de 'date -Is'.

${approved
  ? `Ademas: verifica 'cargo test' y 'cargo clippy --all-targets' sobre TODO el
     workspace (no solo ${CRATE}: esta conveniencia puede haber roto otra). Si
     sale verde, haz commit con mensaje "feat(${CRATE}): ${CONVENIENCIA}" y el
     trailer "Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>".
     Si sale rojo, NO commitees: pon estado "fallida" y dilo.`
  : `NO commitees la implementacion fallida. Deja los ficheros de prueba y el
     contrato (ya estan commiteados aparte) y revierte solo los cambios de
     implementacion con 'git checkout -- <ficheros de stub>' para que el arbol
     quede compilando.`}

${LEDGER_SPEC}
`, { model: 'haiku', effort: 'low', label: `cierre:${SLUG}` })

return {
  conveniencia: CONVENIENCIA,
  crate: CRATE,
  slug: SLUG,
  aprobado: approved,
  interrumpido: false,
  intentos: history.length + (approved ? 1 : 0),
  pruebas: { total: totalTests, ficheros: testFiles.map(f => f.path) },
  checkpoint: LEDGER,
  auditoriaFinal: lastAudit,
  supervisionFinal: lastReview,
  historial: history,
}
