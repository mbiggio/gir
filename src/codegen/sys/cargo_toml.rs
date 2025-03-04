use super::collect_versions;
use crate::{config::Config, env::Env, file_saver::save_to_file, nameutil, version::Version};
use log::info;
use std::{collections::HashMap, fs::File, io::prelude::*};
use toml::{self, value::Table, Value};

pub fn generate(env: &Env) -> String {
    info!("Generating sys Cargo.toml for {}", env.config.library_name);

    let path = env.config.target_path.join("Cargo.toml");

    let mut toml_str = String::new();
    if let Ok(mut file) = File::open(&path) {
        file.read_to_string(&mut toml_str).unwrap();
    }
    let empty = toml_str.trim().is_empty();
    let mut root_table = toml::from_str(&toml_str).unwrap_or_else(|_| Table::new());
    let crate_name = get_crate_name(&env.config, &root_table);

    if empty {
        fill_empty(&mut root_table, env, &crate_name);
    }
    fill_in(&mut root_table, env);

    save_to_file(&path, env.config.make_backup, |w| {
        w.write_all(toml::to_string(&root_table).unwrap().as_bytes())
    });

    crate_name
}

fn fill_empty(root: &mut Table, env: &Env, crate_name: &str) {
    let package_name = nameutil::exported_crate_name(crate_name);

    {
        let package = upsert_table(root, "package");
        set_string(package, "name", package_name);
        set_string(package, "version", "0.0.1");
        set_string(
            package,
            "links",
            nameutil::shared_libs_to_links(&env.namespaces.main().shared_libs),
        );
        set_string(package, "edition", "2018");
    }

    {
        let lib = upsert_table(root, "lib");
        set_string(lib, "name", crate_name);
    }

    let deps = upsert_table(root, "dependencies");
    for ext_lib in &env.config.external_libraries {
        let ext_package = if ext_lib.crate_name == "cairo" {
            format!("{}-sys-rs", ext_lib.crate_name)
        } else if ext_lib.crate_name == "gdk_pixbuf" {
            "gdk-pixbuf-sys".into()
        } else {
            format!("{}-sys", ext_lib.crate_name)
        };
        let dep = upsert_table(deps, &*ext_package);
        if ext_lib.crate_name == "cairo" {
            set_string(dep, "git", "https://github.com/gtk-rs/cairo");
        } else if ext_package.starts_with("sourceview") {
            set_string(dep, "git", "https://github.com/gtk-rs/sourceview");
        } else {
            set_string(dep, "git", "https://github.com/gtk-rs/sys");
        }
    }
}

fn fill_in(root: &mut Table, env: &Env) {
    {
        let package = upsert_table(root, "package");
        set_string(package, "build", "build.rs");
        //set_string(package, "version", "0.2.0");
    }

    {
        let deps = upsert_table(root, "dependencies");
        set_string(deps, "libc", "0.2");
    }

    {
        let build_deps = upsert_table(root, "build-dependencies");
        set_string(build_deps, "system-deps", "2.0");
    }

    {
        let dev_deps = upsert_table(root, "dev-dependencies");
        set_string(dev_deps, "shell-words", "1.0.0");
        set_string(dev_deps, "tempfile", "3");
        unset(dev_deps, "tempdir");
    }

    {
        let features = upsert_table(root, "features");
        let versions = collect_versions(env);
        versions.keys().fold(None::<Version>, |prev, &version| {
            let prev_array: Vec<Value> =
                get_feature_dependencies(version, prev, &env.config.feature_dependencies)
                    .iter()
                    .map(|s| Value::String(s.clone()))
                    .collect();
            features.insert(version.to_feature(), Value::Array(prev_array));
            Some(version)
        });
        features.insert("dox".to_string(), Value::Array(Vec::new()));
    }

    {
        let meta = upsert_table(root, "package");
        let meta = upsert_table(meta, "metadata");
        let meta = upsert_table(meta, "system-deps");

        let ns = env.namespaces.main();
        let lib_name = ns.package_name.as_ref().unwrap();

        let meta = upsert_table(meta, nameutil::lib_name_to_toml(lib_name));
        // Allow both the name and version of a system dep to be overridden by hand
        meta.entry("name")
            .or_insert_with(|| Value::String(lib_name.to_owned()));
        meta.entry("version")
            .or_insert_with(|| Value::String(env.config.min_cfg_version.to_string()));

        // Old version API
        unset(meta, "feature-versions");

        collect_versions(env)
            .iter()
            .filter(|(&v, _)| v > env.config.min_cfg_version)
            .for_each(|(v, lib_version)| {
                let version_section = upsert_table(meta, &v.to_feature());
                // Allow system-deps version for this feature level to be overridden by hand
                version_section
                    .entry("version")
                    .or_insert_with(|| Value::String(lib_version.to_string()));
            });
    }

    {
        // Small trick to prevent having double quotes around it since toml doesn't like having '.'
        let docs_rs_metadata = upsert_table(root, "package");
        let docs_rs_metadata = upsert_table(docs_rs_metadata, "metadata");
        let docs_rs_metadata = upsert_table(docs_rs_metadata, "docs");
        let docs_rs_metadata = upsert_table(docs_rs_metadata, "rs");
        let mut docs_rs_features = env.config.docs_rs_features.clone();
        docs_rs_features.push("dox".to_owned());
        docs_rs_metadata.insert(
            "features".to_string(),
            Value::Array(
                docs_rs_features
                    .into_iter()
                    .map(Value::String)
                    .collect::<Vec<_>>(),
            ),
        );
    }
}

fn get_feature_dependencies(
    version: Version,
    prev_version: Option<Version>,
    feature_dependencies: &HashMap<Version, Vec<String>>,
) -> Vec<String> {
    let mut vec = Vec::with_capacity(10);
    if let Some(v) = prev_version {
        vec.push(v.to_feature());
    }
    if let Some(dependencies) = feature_dependencies.get(&version) {
        vec.extend_from_slice(dependencies);
    }
    vec
}

/// Returns the name of crate being currently generated.
fn get_crate_name(config: &Config, root: &Table) -> String {
    if let Some(&Value::Table(ref lib)) = root.get("lib") {
        if let Some(&Value::String(ref lib_name)) = lib.get("name") {
            //Converting don't needed as library target names cannot contain hyphens
            return lib_name.to_owned();
        }
    }
    if let Some(&Value::Table(ref package)) = root.get("package") {
        if let Some(&Value::String(ref package_name)) = package.get("name") {
            return nameutil::crate_name(package_name);
        }
    }
    return format!("{}_sys", nameutil::crate_name(&config.library_name));
}

fn set_string<S: Into<String>>(table: &mut Table, name: &str, new_value: S) {
    table.insert(name.into(), Value::String(new_value.into()));
}

fn unset(table: &mut Table, name: &str) {
    table.remove(name);
}

fn upsert_table<S: Into<String>>(parent: &mut Table, name: S) -> &mut Table {
    if let Value::Table(ref mut table) = *parent
        .entry(name.into())
        .or_insert_with(|| Value::Table(toml::map::Map::new()))
    {
        table
    } else {
        unreachable!()
    }
}
