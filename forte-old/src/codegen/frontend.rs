use crate::watcher::RouteInfo;
use anyhow::{Context, Result};
use std::path::Path;

/// Generate frontend SSR server code
pub fn generate_frontend_code(project_root: &Path, routes: &[RouteInfo]) -> Result<()> {
    let gen_dir = project_root.join(".generated/frontend");
    std::fs::create_dir_all(&gen_dir).context("Failed to create .generated/frontend")?;

    // Generate ErrorBoundary.tsx
    let error_boundary_ts = crate::templates::error_boundary::ERROR_BOUNDARY_TEMPLATE;
    std::fs::write(gen_dir.join("ErrorBoundary.tsx"), error_boundary_ts)?;

    // Generate server.ts
    let server_ts = generate_ssr_server(routes)?;
    std::fs::write(gen_dir.join("server.ts"), server_ts)?;

    // Generate client.ts (hydration)
    let client_ts = generate_client_hydration();
    std::fs::write(gen_dir.join("client.ts"), client_ts)?;

    // Generate routes.ts (route mapping)
    let routes_ts = generate_routes_mapping(routes)?;
    std::fs::write(gen_dir.join("routes.ts"), routes_ts)?;

    // Create symlink to node_modules for module resolution
    // This allows .generated/frontend/server.ts to find dependencies
    let node_modules_link = project_root.join(".generated/node_modules");
    let frontend_node_modules = project_root.join("frontend/node_modules");

    // Remove existing symlink if it exists
    let _ = std::fs::remove_dir(&node_modules_link);

    // Create symlink (Unix-style, works on macOS and Linux)
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let _ = symlink(&frontend_node_modules, &node_modules_link);
    }

    println!("  ✓ Generated frontend SSR code in .generated/frontend/");

    Ok(())
}

fn generate_ssr_server(_routes: &[RouteInfo]) -> Result<String> {
    let mut output = String::new();

    output.push_str(r#"// [Generated] Do not edit manually
import express from 'express';
import * as React from 'react';
import { renderToString } from 'react-dom/server';
import { RouterProvider } from '../../frontend/src/forte/Router.tsx';
import { routes } from './routes.js';
import type { RouteConfig } from './routes.js';
import { ErrorBoundary } from './ErrorBoundary.tsx';
import { fileURLToPath } from 'url';
import { dirname, resolve } from 'path';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

async function startServer() {
  const app = express();
  const PORT = process.env.SSR_PORT || 0;  // 0 = use any available port
  const RUST_PORT = process.env.RUST_PORT || 8080;

  // Parse form data and JSON
  app.use(express.urlencoded({ extended: true }));
  app.use(express.json());

  // Create Vite dev server for transforming client code
  // Project root is two directories up from .generated/frontend/
  const projectRoot = resolve(__dirname, '../..');
  const vite = await (await import('vite')).createServer({
    server: { middlewareMode: true },
    appType: 'custom',
    root: projectRoot,
    cacheDir: resolve(projectRoot, '.vite'),
  });

  // Use Vite middleware for transforming TS/TSX files
  app.use(vite.middlewares);

  // SSR handler
  app.use(async (req, res) => {
  try {
    // Find matching route
    const route = routes.find(r => {
      if (r.pattern instanceof RegExp) {
        return r.pattern.test(req.path);
      }
      return r.pattern === req.path;
    });

    if (!route) {
      return res.status(404).send('Page not found');
    }

    // Fetch data from Rust backend (via /__wasm/ prefix to avoid proxy loop)
    const backendUrl = `http://localhost:${RUST_PORT}/__wasm${req.path}`;
    let response;

    if (req.method === 'POST') {
      // POST request - send form data as JSON to backend
      response = await fetch(backendUrl, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(req.body),
      });
    } else {
      // GET request
      response = await fetch(backendUrl);
    }

    if (!response.ok) {
      const errorData = await response.json();

      // System error (500) - show error page
      if (response.status === 500) {
        // Check if client wants JSON
        const acceptHeader = req.get('Accept') || '';
        if (acceptHeader.includes('application/json')) {
          return res.status(500).json(errorData);
        }

        // Try to render error.tsx
        try {
          let ErrorPage: any = null;
          try {
            const rootErrorModule = await import('../../frontend/src/app/error.tsx');
            ErrorPage = rootErrorModule.default;
          } catch (e) {
            // No error.tsx, return plain error
            return res.status(500).send(`
              <html>
                <body>
                  <h1>Internal Server Error</h1>
                  <p>${errorData.message || 'An unexpected error occurred'}</p>
                </body>
              </html>
            `);
          }

          const errorProps = {
            status: 500,
            message: errorData.message || 'Internal Server Error',
            error: errorData
          };

          let RootLayout: any = null;
          try {
            const rootLayoutModule = await import('../../frontend/src/app/layout.tsx');
            RootLayout = rootLayoutModule.default;
          } catch (e) {
            // No root layout
          }

          let component = React.createElement(ErrorPage, errorProps);
          if (RootLayout) {
            component = React.createElement(RootLayout, { children: component });
          }

          const html = renderToString(component);

          if (RootLayout) {
            const htmlWithScripts = html.replace(
              '</body>',
              `<script>window.__INITIAL_PROPS__ = ${JSON.stringify(errorProps)};</script></body>`
            );
            return res.status(500).send('<!DOCTYPE html>\\n' + htmlWithScripts);
          } else {
            return res.status(500).send(`
              <!DOCTYPE html>
              <html>
                <head><title>Error 500</title></head>
                <body>
                  <div id="root">${html}</div>
                  <script>window.__INITIAL_PROPS__ = ${JSON.stringify(errorProps)};</script>
                </body>
              </html>
            `);
          }
        } catch (renderError) {
          console.error('[SSR] Error rendering error page:', renderError);
          return res.status(500).json(errorData);
        }
      }

      // Other errors (not 500) - this shouldn't happen in normal flow
      // as business logic errors should be returned as 200 OK with Props enum
      return res.status(response.status).json(errorData);
    }

    let pageProps = await response.json();

    // Handle ActionResult for POST requests
    if (req.method === 'POST' && pageProps.type) {
      if (pageProps.type === 'redirect') {
        return res.redirect(pageProps.url);
      } else if (pageProps.type === 'render') {
        pageProps = pageProps.props;
      }
    }

    // Check if client wants JSON (client-side navigation)
    const acceptHeader = req.get('Accept') || '';
    if (acceptHeader.includes('application/json')) {
      return res.json(pageProps);
    }

    // Dynamically import the page component
    const pageModule = await import(route.componentPath);
    const Page = pageModule.default;

    // Import layout if it exists
    let Layout: any = null;
    try {
      const layoutPath = route.componentPath.replace('/page.tsx', '/layout.tsx');
      const layoutModule = await import(layoutPath);
      Layout = layoutModule.default;
    } catch (e) {
      // No layout, that's ok
    }

    // Try importing error.tsx for this route
    let ErrorPage: any = null;
    try {
      const errorPath = route.componentPath.replace('/page.tsx', '/error.tsx');
      const errorModule = await import(errorPath);
      ErrorPage = errorModule.default;
    } catch (e) {
      // No custom error page
    }

    // Also try root layout
    let RootLayout: any = null;
    try {
      const rootLayoutModule = await import('../../frontend/src/app/layout.tsx');
      RootLayout = rootLayoutModule.default;
    } catch (e) {
      // No root layout
    }

    // Build component tree with layouts
    let component = React.createElement(Page, pageProps);

    if (Layout) {
      component = React.createElement(Layout, { children: component });
    }

    // Wrap with error boundary if custom error page exists
    if (ErrorPage) {
      component = React.createElement(
        ErrorBoundary,
        { fallback: ErrorPage },
        component
      );
    } else {
      // Use default error boundary
      component = React.createElement(
        ErrorBoundary,
        {},
        component
      );
    }

    if (RootLayout) {
      component = React.createElement(RootLayout, { children: component });
    }

    // Wrap with RouterProvider for SSR
    component = React.createElement(
      RouterProvider,
      { initialPath: req.path },
      component
    );

    // Render to string
    const html = renderToString(component);

    // If RootLayout exists, it returns complete HTML structure
    if (RootLayout) {
      // Inject scripts before closing body tag
      const htmlWithScripts = html.replace(
        '</body>',
        `<script>window.__INITIAL_PROPS__ = ${JSON.stringify(pageProps)};</script><script type="module" src="/.generated/frontend/client.ts"></script></body>`
      );
      res.send('<!DOCTYPE html>\\n' + htmlWithScripts);
    } else {
      // No RootLayout, wrap in default HTML structure
      res.send(`
        <!DOCTYPE html>
        <html lang="en">
          <head>
            <meta charset="UTF-8" />
            <meta name="viewport" content="width=device-width, initial-scale=1.0" />
            <title>Forte App</title>
          </head>
          <body>
            <div id="root">${html}</div>
            <script>
              window.__INITIAL_PROPS__ = ${JSON.stringify(pageProps)};
            </script>
            <script type="module" src="/.generated/frontend/client.js"></script>
          </body>
        </html>
      `);
    }
  } catch (err) {
    console.error('SSR Error:', err);
    res.status(500).send('Internal Server Error: ' + err.message);
  }
});

  const server = app.listen(PORT, () => {
    const actualPort = server.address().port;
    // Output port to stdout for CLI to capture (must be on a single line)
    console.log(`SSR_PORT=${actualPort}`);
    console.error(`[SSR] SSR server listening on http://localhost:${actualPort}`);
  });
}

startServer().catch(err => {
  console.error('Failed to start server:', err);
  process.exit(1);
});
"#);

    Ok(output)
}

fn generate_client_hydration() -> String {
    r#"// [Generated] Client-side hydration
import * as React from 'react';
import { hydrateRoot, createRoot } from 'react-dom/client';
import { RouterProvider, ErrorOverlay } from '../../frontend/src/forte/Router.tsx';
import { ErrorBoundary } from './ErrorBoundary.tsx';
import { routes } from './routes.js';

let root = null;

async function renderApp(pageProps = null) {
  try {
    // Use provided props or get from window
    const initialProps = pageProps || window.__INITIAL_PROPS__ || {};

    // Find matching route
    const currentPath = window.location.pathname;
    const route = routes.find(r => {
      if (r.pattern instanceof RegExp) {
        return r.pattern.test(currentPath);
      }
      return r.pattern === currentPath;
    });

    if (!route) {
      console.error('[Forte] No route found for', currentPath);
      return;
    }

    // Dynamically import the page component
    const pageModule = await import(route.componentPath);
    const Page = pageModule.default;

    // Import layout if it exists
    let Layout = null;
    try {
      const layoutPath = route.componentPath.replace('/page.tsx', '/layout.tsx');
      const layoutModule = await import(layoutPath);
      Layout = layoutModule.default;
    } catch (e) {
      // No route-specific layout
    }

    // Try importing error.tsx for this route
    let ErrorPage = null;
    try {
      const errorPath = route.componentPath.replace('/page.tsx', '/error.tsx');
      const errorModule = await import(errorPath);
      ErrorPage = errorModule.default;
    } catch (e) {
      // No custom error page
    }

    // Import root layout
    let RootLayout = null;
    try {
      const rootLayoutModule = await import('../../frontend/src/app/layout.tsx');
      RootLayout = rootLayoutModule.default;
    } catch (e) {
      // No root layout
    }

    // Build component tree (same as server)
    let component = React.createElement(Page, initialProps);

    if (Layout) {
      component = React.createElement(Layout, { children: component });
    }

    // Wrap with error boundary if custom error page exists
    if (ErrorPage) {
      component = React.createElement(
        ErrorBoundary,
        { fallback: ErrorPage },
        component
      );
    } else {
      // Use default error boundary
      component = React.createElement(
        ErrorBoundary,
        {},
        component
      );
    }

    if (RootLayout) {
      component = React.createElement(RootLayout, { children: component });
    }

    // Wrap with RouterProvider for client-side navigation
    component = React.createElement(
      RouterProvider,
      { initialPath: currentPath },
      component
    );

    // Add ErrorOverlay for development
    const appWithOverlay = React.createElement(
      React.Fragment,
      null,
      component,
      React.createElement(ErrorOverlay)
    );

    // Hydrate or render the root
    const rootElement = document.getElementById('root');
    if (rootElement) {
      if (!root) {
        // Initial hydration
        root = hydrateRoot(rootElement, appWithOverlay);
        console.log('[Forte] Hydration complete');
      } else {
        // Client-side navigation - re-render
        root.render(appWithOverlay);
        console.log('[Forte] Navigation render complete');
      }
    } else {
      console.error('[Forte] Root element not found');
    }
  } catch (err) {
    console.error('[Forte] Render failed:', err);
  }
}

async function hydrate() {
  // Initial hydration
  await renderApp();

  // Listen for client-side navigation
  window.addEventListener('forte:navigate', async (e) => {
    const { pageProps } = e.detail;
    await renderApp(pageProps);
  });
}

// Start hydration when DOM is ready
if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', hydrate);
} else {
  hydrate();
}
"#
    .to_string()
}

fn generate_routes_mapping(routes: &[RouteInfo]) -> Result<String> {
    let mut output = String::new();

    output.push_str("// [Generated] Route mappings\n\n");
    output.push_str("export interface RouteConfig {\n");
    output.push_str("  pattern: string | RegExp;\n");
    output.push_str("  componentPath: string;\n");
    output.push_str("}\n\n");
    output.push_str("export const routes: RouteConfig[] = [\n");

    for route in routes {
        let route_path = extract_route_path(&route.props_path)?;
        let url_pattern = convert_to_url_pattern(&route_path);
        let component_path = get_component_path(&route.frontend_dir)?;

        output.push_str("  {\n");
        output.push_str(&format!("    pattern: {},\n", url_pattern));
        output.push_str(&format!("    componentPath: '{}',\n", component_path));
        output.push_str("  },\n");
    }

    output.push_str("];\n");

    Ok(output)
}

/// Extract route path from props.rs path
fn extract_route_path(props_path: &Path) -> Result<String> {
    let path_str = props_path.to_str().context("Invalid UTF-8 in path")?;

    if let Some(routes_idx) = path_str.find("routes/") {
        let after_routes = &path_str[routes_idx + 7..];
        if let Some(props_idx) = after_routes.find("/props.rs") {
            return Ok(after_routes[..props_idx].to_string());
        }
    }

    anyhow::bail!("Could not extract route path from: {}", path_str)
}

/// Convert route path to URL pattern (for matching)
fn convert_to_url_pattern(route_path: &str) -> String {
    // For simple routes, use exact match
    // For dynamic routes, use regex
    if route_path.contains('_') {
        // Dynamic route - create regex pattern
        let pattern = route_path
            .split('/')
            .map(|segment| {
                if segment.starts_with('_') && segment.ends_with('_') && segment.len() > 2 {
                    "[^/]+".to_string() // Match any non-slash characters
                } else if segment == "index" {
                    "".to_string()
                } else {
                    regex::escape(segment)
                }
            })
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("/");

        format!("new RegExp('^/{}/?$')", pattern)
    } else {
        // Static route - exact match
        let url = if route_path == "index" {
            "/".to_string()
        } else {
            format!("/{}", route_path)
        };
        format!("'{}'", url)
    }
}

/// Get component path relative to .generated/frontend
fn get_component_path(frontend_dir: &Path) -> Result<String> {
    let path_str = frontend_dir.to_str().context("Invalid UTF-8 in path")?;

    // Extract the route path from frontend_dir
    // frontend_dir is like: /path/to/frontend/src/app/product/[id]
    // We want: ../../frontend/src/app/product/[id]/page.tsx

    if let Some(frontend_idx) = path_str.rfind("/frontend/") {
        let from_frontend = &path_str[frontend_idx + 1..]; // "frontend/src/app/product/[id]"
        return Ok(format!("../../{}/page.tsx", from_frontend));
    }

    anyhow::bail!("Could not extract component path from: {}", path_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_pattern_static() {
        assert_eq!(convert_to_url_pattern("index"), "'/'");
        assert_eq!(convert_to_url_pattern("about"), "'/about'");
    }

    #[test]
    fn test_url_pattern_dynamic() {
        let pattern = convert_to_url_pattern("product/_id_");
        assert!(pattern.contains("RegExp"));
        assert!(pattern.contains("[^/]+"));
    }
}
