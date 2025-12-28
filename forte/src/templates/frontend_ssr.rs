// Node.js SSR server templates

pub const NODE_SSR_SERVER: &str = r#"// [Generated] SSR server
import express from 'express';
import * as React from 'react';
import { renderToString } from 'react-dom/server';

const app = express();
app.use(express.json());

// Import all page components dynamically
const pages: Record<string, any> = {
  '/': () => import('../../frontend/src/app/index/page.js'),
};

app.post('/__ssr', async (req, res) => {
  try {
    const path = req.query.path as string || '/';
    const pageProps = req.body;

    console.log(`[SSR] Rendering ${path} with props:`, pageProps);

    // Load the page component
    const pageLoader = pages[path];
    if (!pageLoader) {
      console.error(`[SSR] No page component found for path: ${path}`);
      return res.status(404).send('Page not found');
    }

    const pageModule = await pageLoader();
    const PageComponent = pageModule.default;

    // Render to string
    const html = renderToString(React.createElement(PageComponent, pageProps));

    // Wrap in HTML document
    const fullHtml = `
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Forte App</title>
</head>
<body>
  <div id="root">${html}</div>
  <script>
    window.__INITIAL_PROPS__ = ${JSON.stringify(pageProps)};
  </script>
  <script type="module" src="/client/main.js"></script>
</body>
</html>
`;

    res.send(fullHtml);
  } catch (error) {
    console.error('[SSR] Error:', error);
    res.status(500).send('SSR Error');
  }
});

const PORT = process.env.PORT || 5173;
app.listen(PORT, () => {
  console.log(`[SSR] Node.js SSR server listening on http://localhost:${PORT}`);
});
"#;

pub const VITE_SSR_CONFIG: &str = r#"import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import path from 'path'

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src'),
    },
  },
  build: {
    ssr: true,
    outDir: '../.generated/frontend',
    rollupOptions: {
      input: './src/app/index/page.tsx',
    },
  },
})
"#;

pub const FRONTEND_ENTRY_CLIENT: &str = r#"// [Generated] Client-side hydration
import React from 'react';
import ReactDOM from 'react-dom/client';

// This will be dynamically replaced based on the route
const root = document.getElementById('root');
if (root) {
  ReactDOM.hydrateRoot(root, React.createElement('div', null, root.innerHTML));
}
"#;
