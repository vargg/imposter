//! Request scripts in Rhai, run in a sandbox: no files, network, clock or environment; the
//! host gives time and randomness. The engine and its API grow here; nothing else in the
//! workspace embeds Rhai.

#[cfg(test)]
mod tests {
    /// The engine builds with the features the workspace pins (no modules, no clock).
    #[test]
    fn rhai_evaluates_without_modules_or_time() {
        let engine = rhai::Engine::new();
        assert_eq!(engine.eval::<i64>("let x = 40; x + 2").unwrap(), 42);
        assert!(
            engine.compile("import \"x\" as y;").is_err(),
            "no module system"
        );
        assert!(engine.eval::<i64>("timestamp()").is_err(), "no clock");
    }
}
