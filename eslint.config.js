import js from '@eslint/js';
import tseslint from 'typescript-eslint';
export default tseslint.config(
  { ignores: ['**/dist/**', '**/schema.d.ts'] },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  { files: ['**/*.{ts,tsx}'], rules: { '@typescript-eslint/no-explicit-any': 'error', '@typescript-eslint/consistent-type-imports': 'error' } },
  { files: ['tooling/*.mjs'], languageOptions: { globals: { console: 'readonly', process: 'readonly', Buffer: 'readonly', setTimeout: 'readonly', fetch: 'readonly', AbortSignal: 'readonly', URL: 'readonly', crypto:'readonly',AbortController:'readonly',TextDecoder:'readonly',clearTimeout:'readonly' } } }
);
