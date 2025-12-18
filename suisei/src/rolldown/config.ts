export function generateRolldownConfig(inputFile: string, outputFile: string): string {
  return `export default {
  input: '${inputFile}',
  external: /wasi:.*/,
  output: {
    file: '${outputFile}',
    format: 'esm',
    inlineDynamicImports: true,
  },
};
`;
}
