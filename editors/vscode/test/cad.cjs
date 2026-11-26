const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const Module = require('node:module');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');

test('CAD UI previews before approval and recovers an accepted lost response', async () => {
    const root = await fs.mkdtemp(path.join(os.tmpdir(), 'cad-ui-'));
    const previous = process.env.LUMA_CAD_PROJECT_KEY;
    process.env.LUMA_CAD_PROJECT_KEY = '0'.repeat(64);
    try {
        const fixture = path.join(root, 'fixture.cjs');
        await fs.writeFile(fixture, `
            const fs = require('node:fs');
            const path = require('node:path');
            const [, file, , action, ...rest] = process.argv;
            fs.appendFileSync(path.join(path.dirname(file), 'calls'), action + '\\n');
            const out = {protocol:'luma.cad.core-project/1', schema_version:1};
            if (action === 'preview') {
                fs.writeFileSync(rest[4], 'prepared token', {flag:'wx'});
                out.result = {project_revision:'1', candidate_changed:true};
            } else if (action === 'apply') {
                if (fs.readFileSync(rest[0], 'utf8') !== 'prepared token') process.exit(3);
                fs.writeFileSync(path.join(path.dirname(file), 'accepted'), rest[1]);
                // Lose the response after acceptance.
                process.exit(1);
            } else if (action === 'receipt') {
                if (fs.readFileSync(path.join(path.dirname(file), 'accepted'), 'utf8') !== rest[1]) process.exit(4);
                out.result = {project_revision:'2'};
            } else {
                process.exit(5);
            }
            console.log(JSON.stringify(out));
        `);
        let answer;
        let selections = 0;
        let inputs = 0;
        const shown = [];
        const errors = [];
        const uuid = '00000000-0000-4000-8000-000000000002';
        const vscode = {
            workspace: {
                isTrusted: true,
                getConfiguration: () => ({get: () => process.execPath}),
                openTextDocument: async descriptor => descriptor,
            },
            env: {},
            window: {
                showQuickPick: async () => 'command',
                showOpenDialog: async () => [{
                    scheme: 'file',
                    fsPath: selections++ % 2 === 0 ? fixture : path.join(root, 'source.json'),
                }],
                showInputBox: async () => (inputs++ % 3 === 2 ? '1' : uuid),
                showTextDocument: async document => shown.push(JSON.parse(document.content)),
                showInformationMessage: async () => {
                    assert.equal(shown.at(-1).candidate_changed, true);
                    return answer;
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
        const calls = path.join(root, 'calls');
        assert.equal(await fs.readFile(calls, 'utf8'), 'preview\n');

        answer = 'Apply';
        await preview();
        assert.equal(await fs.readFile(calls, 'utf8'), 'preview\npreview\napply\nreceipt\n');
        assert.equal(shown.at(-1).project_revision, '2');
        assert.match(shown.at(-1).operation, /^[0-9a-f]{32}$/);
        assert.deepEqual(errors, []);
    } finally {
        if (previous === undefined) delete process.env.LUMA_CAD_PROJECT_KEY;
        else process.env.LUMA_CAD_PROJECT_KEY = previous;
        await fs.rm(root, {recursive: true, force: true});
    }
});
