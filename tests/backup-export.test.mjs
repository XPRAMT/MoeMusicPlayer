import test from 'node:test';
import assert from 'node:assert/strict';
import { buildBackupSelection, getBackupExportProblem } from '../src/lib/backup-export.js';

const base = { library: true, settings: true, accounts: false, favourites: true, passphrase: '', confirm: '' };

test('nothing ticked cannot be exported', () => {
  assert.equal(getBackupExportProblem({ ...base, library: false, settings: false, favourites: false }), 'nothing');
  assert.equal(getBackupExportProblem(base), null);
});

test('accounts need a passphrase of at least 8 characters that is typed twice', () => {
  const accounts = { ...base, accounts: true };
  assert.equal(getBackupExportProblem(accounts), 'short');
  assert.equal(getBackupExportProblem({ ...accounts, passphrase: '1234567', confirm: '1234567' }), 'short');
  assert.equal(getBackupExportProblem({ ...accounts, passphrase: '12345678', confirm: '1234567' }), 'mismatch');
  assert.equal(getBackupExportProblem({ ...accounts, passphrase: '12345678', confirm: '12345678' }), null);
  // Only the accounts need it.
  assert.equal(getBackupExportProblem({ ...base, passphrase: '1' }), null);
  // Accounts alone are a valid backup.
  assert.equal(getBackupExportProblem({ library: false, settings: false, favourites: false, accounts: true, passphrase: 'abcdefgh', confirm: 'abcdefgh' }), null);
});

test('the passphrase is only sent with the accounts', () => {
  assert.equal(buildBackupSelection({ ...base, passphrase: 'secret-pass' }).passphrase, null);
  assert.equal(buildBackupSelection({ ...base, accounts: true, passphrase: 'secret-pass' }).passphrase, 'secret-pass');
});
