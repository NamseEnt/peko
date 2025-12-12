use crate::{
    config::{Framework, ProjectConfig},
    generators::templates::{hono, package_json, rolldown_config, wit_component, wit_deps},
    utils::command::CommandExecutor,
};
use color_eyre::Result;
use std::path::Path;
use tokio::fs;

pub struct TypeScriptGenerator;

impl TypeScriptGenerator {
    pub async fn generate(project_path: &Path, config: &ProjectConfig) -> Result<()> {
        println!("📦 Creating project directory...");
        fs::create_dir_all(project_path).await?;

        println!("📄 Creating package.json...");
        let package_json_content = package_json::generate(config);
        let package_json_path = project_path.join("package.json");
        fs::write(package_json_path, package_json_content).await?;

        println!("📄 Creating tsconfig.json...");
        let tsconfig_content = Self::generate_tsconfig();
        let tsconfig_path = project_path.join("tsconfig.json");
        fs::write(tsconfig_path, tsconfig_content).await?;

        println!("📄 Creating rolldown.config.mjs...");
        let rolldown_content = rolldown_config::generate();
        let rolldown_path = project_path.join("rolldown.config.mjs");
        fs::write(rolldown_path, rolldown_content).await?;

        println!("📄 Creating .gitignore...");
        let gitignore_content = Self::generate_gitignore();
        let gitignore_path = project_path.join(".gitignore");
        fs::write(gitignore_path, gitignore_content).await?;

        println!("📄 Creating WIT interface files...");
        let wit_path = project_path.join("wit");
        fs::create_dir_all(&wit_path).await?;

        let wit_component_content = wit_component::generate();
        let wit_component_path = wit_path.join("component.wit");
        fs::write(wit_component_path, wit_component_content).await?;

        let wit_deps_path = wit_path.join("deps");
        fs::create_dir_all(&wit_deps_path).await?;

        let deps = wit_deps::WitDeps::get_all_deps();
        for (rel_path, content) in deps {
            let parts: Vec<&str> = rel_path.split('/').collect();
            let dir_name = parts[0];
            let file_name = parts[1];

            let dep_dir = wit_deps_path.join(dir_name);
            fs::create_dir_all(&dep_dir).await?;

            let dep_file = dep_dir.join(file_name);
            fs::write(dep_file, content).await?;
        }

        let src_path = project_path.join("src");
        fs::create_dir_all(&src_path).await?;

        println!("📄 Creating server files...");
        match config.framework {
            Framework::Hono => {
                let index_content = hono::generate_index();
                let index_path = src_path.join("index.ts");
                fs::write(index_path, index_content).await?;

                let component_content = hono::generate_component();
                let component_path = src_path.join("component.ts");
                fs::write(component_path, component_content).await?;
            }
        }

        println!("📥 Installing dependencies...");

        let pm_cmd = config.package_manager.command();

        println!("  Installing {}...", config.framework);
        let mut install_args = config.package_manager.install_args();
        install_args.push(config.framework.package_name());

        CommandExecutor::run(pm_cmd, &install_args, project_path).await?;

        println!("  Installing development dependencies...");
        let mut install_dev_args = config.package_manager.install_dev_args();
        install_dev_args.push("typescript");
        install_dev_args.push("@types/node");

        for pkg in config.framework.additional_packages() {
            install_dev_args.push(pkg);
        }

        CommandExecutor::run(pm_cmd, &install_dev_args, project_path).await?;

        Ok(())
    }

    fn generate_tsconfig() -> String {
        r#"{
  "compilerOptions": {
    "target": "ES2022",
    "module": "nodenext",
    "moduleResolution": "bundler",
    "esModuleInterop": true,
    "strict": true,
    "skipLibCheck": true,
    "forceConsistentCasingInFileNames": true,
    "resolveJsonModule": true,
    "outDir": "./dist",
    "rootDir": "./src"
  },
  "include": ["src/**/*"],
  "exclude": ["node_modules"]
}
"#
        .to_string()
    }

    fn generate_gitignore() -> String {
        r#"node_modules/
dist/
.env
.DS_Store
*.log
*.wasm
"#
        .to_string()
    }
}
