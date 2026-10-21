export const BACKUP_PASSPHRASE_MIN_LENGTH = 8;

/**
 * Why the export panel cannot export yet, or null when it can.
 * @param {{ library: boolean, settings: boolean, accounts: boolean, favourites: boolean, passphrase: string, confirm: string }} input
 * @returns {null | 'nothing' | 'short' | 'mismatch'}
 */
export function getBackupExportProblem(input) {
  if (!(input.library || input.settings || input.accounts || input.favourites)) return 'nothing';
  if (!input.accounts) return null;
  if ([...input.passphrase].length < BACKUP_PASSPHRASE_MIN_LENGTH) return 'short';
  if (input.passphrase !== input.confirm) return 'mismatch';
  return null;
}

/**
 * The selection sent to the backend; the passphrase only travels with the accounts.
 * @param {{ library: boolean, settings: boolean, accounts: boolean, favourites: boolean, passphrase: string }} input
 */
export function buildBackupSelection(input) {
  return {
    library: input.library,
    settings: input.settings,
    accounts: input.accounts,
    favourites: input.favourites,
    passphrase: input.accounts ? input.passphrase : null,
  };
}
