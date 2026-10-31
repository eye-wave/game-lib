#[cfg(feature = "generate-error-docs")]
use errors::ALL_ERRORS;

fn main() {
    #[cfg(feature = "generate-error-docs")]
    {
        let args: Vec<String> = std::env::args().collect();
        let path = args.get(1).map(|s| s.as_str()).unwrap();
        generate_html(path).expect("Failed to generate error documentation");
    }
}

#[cfg(feature = "generate-error-docs")]
pub fn generate_html(path: impl AsRef<std::path::Path>) -> std::io::Result<()> {
    use std::fmt::Write;

    let mut html = String::from(
        "<!DOCTYPE html>\n\
         <html>\n\
         <head>\n\
         <meta charset=\"utf-8\">\n\
         <title>Error Reference</title>\n\
         <style>\n\
         table { border-collapse: collapse; width: 100%; }\n\
         th, td { border: 1px solid #ddd; padding: 8px; text-align: left; }\n\
         th { background-color: #f2f2f2; }\n\
         td.code { text-align: right; }\n\
         code { background-color: #f4f4f4; padding: 2px 4px; border-radius: 4px; }\n\
         </style>\n\
         </head>\n\
         <body>\n\
         <h1>Error Reference</h1>\n\
         <p>Automatically generated error documentation.</p>\n\
         <table>\n\
         <thead>\n\
         <tr><th>Code</th><th>Module</th><th>Name</th><th>Description</th></tr>\n\
         </thead>\n\
         <tbody>\n",
    );

    for error in ALL_ERRORS {
        let escaped_description = error
            .description
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;");

        writeln!(
            &mut html,
            "<tr><td class=\"code\">{}</td><td>{}</td><td><code>{}</code></td><td>{}</td></tr>",
            error.code, error.module, error.name, escaped_description
        )
        .unwrap();
    }

    html.push_str("</tbody>\n</table>\n</body>\n</html>");

    std::fs::write(path, html)
}
