use std::collections::HashMap;

pub trait Lookup {
    fn get(&self, key: &str) -> Option<String>;
}

impl Lookup for HashMap<String, String> {
    fn get(&self, key: &str) -> Option<String> {
        HashMap::get(self, key).cloned()
    }
}

pub struct ProcessEnv;

impl Lookup for ProcessEnv {
    fn get(&self, key: &str) -> Option<String> {
        std::env::var(key).ok()
    }
}

pub struct Inputs {
    pub ctx: HashMap<String, String>,
    pub env: Box<dyn Lookup>,
    pub secret: Box<dyn Lookup>,
}

impl Default for Inputs {
    fn default() -> Self {
        Self::new()
    }
}

impl Inputs {
    pub fn new() -> Self {
        Inputs {
            ctx: HashMap::new(),
            env: Box::new(HashMap::<String, String>::new()),
            secret: Box::new(HashMap::<String, String>::new()),
        }
    }
    pub fn with_ctx(mut self, key: &str, value: &str) -> Self {
        self.ctx.insert(key.to_string(), value.to_string());
        self
    }
    pub fn with_env(mut self, lookup: impl Lookup + 'static) -> Self {
        self.env = Box::new(lookup);
        self
    }
    pub fn with_secret(mut self, lookup: impl Lookup + 'static) -> Self {
        self.secret = Box::new(lookup);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn hashmap_is_a_lookup() {
        let m: HashMap<String, String> = HashMap::from([("A".to_string(), "1".to_string())]);
        assert_eq!(Lookup::get(&m, "A"), Some("1".to_string()));
        assert_eq!(Lookup::get(&m, "B"), None);
    }

    #[test]
    fn process_env_reads_environment() {
        std::env::set_var("MEDL_TEST_VAR_7", "hello");
        assert_eq!(ProcessEnv.get("MEDL_TEST_VAR_7"), Some("hello".to_string()));
        assert_eq!(ProcessEnv.get("MEDL_DEFINITELY_UNSET_VAR"), None);
    }

    #[test]
    fn inputs_builder_sets_ctx_env_and_secret() {
        let inputs = Inputs::new()
            .with_ctx("platform", "mobile")
            .with_env(HashMap::from([("HOST".to_string(), "h".to_string())]))
            .with_secret(HashMap::from([("KEY".to_string(), "k".to_string())]));
        assert_eq!(
            inputs.ctx.get("platform").map(String::as_str),
            Some("mobile")
        );
        assert_eq!(inputs.env.get("HOST"), Some("h".to_string()));
        assert_eq!(inputs.secret.get("KEY"), Some("k".to_string()));
        assert_eq!(Inputs::new().env.get("HOST"), None);
    }
}
