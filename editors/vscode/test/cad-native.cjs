const assert = require('node:assert/strict');
const {execFileSync, spawn} = require('node:child_process');
const {randomBytes} = require('node:crypto');
const fs = require('node:fs/promises');
const Module = require('node:module');
const os = require('node:os');
const path = require('node:path');
const {createInterface} = require('node:readline');
const test = require('node:test');

const executable = process.env.LUMA_CAD_PROJECT_CLI;
const repo = path.resolve(__dirname, '../../..');
const project = '00000000-0000-4000-8000-000000000001';
const part = '00000000-0000-4000-8000-000000000002';

test('VS Code, CLI and MCP share the native candidate and accepted receipt', {
    skip: !executable && 'Set LUMA_CAD_PROJECT_CLI to the composed Core CLI',
    timeout: 180_000,
}, async () => {
    const directory = await fs.mkdtemp(path.join(os.tmpdir(), 'cad-native-clients-'));
    const database = path.join(directory, 'project.db');
    const token = path.join(directory, 'cli.token');
    const source = path.join(repo, 'examples/cad/plate-width-command.json');
    const previousKey = process.env.LUMA_CAD_PROJECT_KEY;
    process.env.LUMA_CAD_PROJECT_KEY = randomBytes(32).toString('hex');
    const call = (...args) => JSON.parse(execFileSync(executable, [database, project, ...args], {
        encoding: 'utf8', timeout: 30_000,
    })).result;
    let bridge;
    let lines;
    try {
        call('init', part, '1'.repeat(32));
        call('preview', 'recipe', part, '1', path.join(repo, 'examples/cad/plate-geometry.json'), token);
        assert.equal(call('apply', token, '2'.repeat(32)).project_revision, '2');
        const cli = call('preview', 'command', part, '2', source, path.join(directory, 'command.token'));

        bridge = spawn('python3', ['-B', path.join(repo, 'tools/cad/project-mcp.py'),
            executable, database, project], {stdio: ['pipe', 'pipe', 'inherit']});
        lines = createInterface({input: bridge.stdout})[Symbol.asyncIterator]();
        let id = 0;
        async function rpc(method, params) {
            bridge.stdin.write(JSON.stringify({jsonrpc: '2.0', id: ++id, method, params}) + '\n');
            const message = await lines.next();
            assert.equal(message.done, false);
            const response = JSON.parse(message.value);
            assert.equal(response.id, id);
            assert.ok(response.result, message.value);
            return response.result;
        }
        async function tool(name, args) {
            const response = await rpc('tools/call', {name, arguments: args});
            assert.equal(response.isError, false, JSON.stringify(response.content));
            return JSON.parse(response.content[0].text);
        }
        await rpc('initialize', {protocolVersion: '2025-03-26'});
        bridge.stdin.write('{"jsonrpc":"2.0","method":"notifications/initialized"}\n');
        const mcp = await tool('prepare_commands', {
            part, expected_project_revision: '2', bundle: JSON.parse(await fs.readFile(source, 'utf8')),
        });
        const {candidate, ...mcpPreview} = mcp;
        assert.deepEqual(mcpPreview, cli);
        assert.deepEqual(await tool('preview', {candidate}), cli);

        let selections = 0;
        const shown = [];
        const errors = [];
        const fields = [project, part, '2'];
        const vscode = {
            workspace: {
                isTrusted: true,
                getConfiguration: () => ({get: () => executable}),
                openTextDocument: async descriptor => descriptor,
            },
            env: {},
            window: {
                showQuickPick: async () => 'command',
                showOpenDialog: async () => [{scheme: 'file', fsPath: [database, source][selections++]}],
                showInputBox: async () => fields.shift(),
                showTextDocument: async document => shown.push(JSON.parse(document.content)),
                showInformationMessage: async () => {
                    assert.deepEqual(shown[0], cli);
                    return 'Apply';
                },
                showErrorMessage: async error => errors.push(error),
            },
        };
        const load = Module._load;
        let preview;
        try {
            Module._load = (name, ...args) => name === 'vscode' ? vscode : load(name, ...args);
            ({previewCadBundle: preview} = require('../dist/cad.js'));
        } finally {
            Module._load = load;
        }
        await preview();
        assert.deepEqual(errors, []);
        assert.equal(shown.length, 2);
        const {operation, ...receipt} = shown[1];
        assert.equal(receipt.project_revision, '3');
        assert.deepEqual(call('receipt', part, operation), receipt);
        assert.deepEqual(await tool('receipt', {part, operation}), receipt);
        assert.equal(call('recipe-info', part).outputs.width.Length, 0.12);
        const stale = await rpc('tools/call', {
            name: 'commit', arguments: {candidate, operation: '3'.repeat(32)},
        });
        assert.equal(stale.isError, true);
        assert.equal(JSON.parse(stale.content[0].text).code, 'Conflict');
    } finally {
        bridge?.stdin.end();
        bridge?.kill();
        if (previousKey === undefined) delete process.env.LUMA_CAD_PROJECT_KEY;
        else process.env.LUMA_CAD_PROJECT_KEY = previousKey;
        await fs.rm(directory, {recursive: true, force: true});
    }
});
