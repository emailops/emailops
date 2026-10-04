//! Pure: the human-readable part of a shared document's messages, in the
//! sender's UI language (the recipient may not have EmailOps and reads it in
//! any mail client). The machine part is the `.eodoc` attachment.

use crate::services::i18n::Language;

pub fn subject(language: Language, title: &str) -> String {
    match language {
        Language::En => format!("{title} (EmailOps shared document)"),
        Language::Es => format!("{title} (documento compartido de EmailOps)"),
        Language::Fr => format!("{title} (document partagé EmailOps)"),
        Language::De => format!("{title} (geteiltes EmailOps-Dokument)"),
    }
}

/// The invitation text: who shared what, and that it can only be edited in EmailOps.
pub fn invitation(language: Language, sharer: &str, title: &str) -> String {
    match language {
        Language::En => format!(
            "{sharer} shared \"{title}\" with you in EmailOps.\n\n\
             Open EmailOps to edit it together: changes travel between the people \
             sharing it as email, with no server in between. Without EmailOps you \
             can read the copy below, but not edit it."
        ),
        Language::Es => format!(
            "{sharer} ha compartido «{title}» contigo en EmailOps.\n\n\
             Ábrelo en EmailOps para editarlo juntos: los cambios viajan por email \
             entre quienes lo comparten, sin ningún servidor intermedio. Sin EmailOps \
             puedes leer la copia de abajo, pero no editarla."
        ),
        Language::Fr => format!(
            "{sharer} a partagé « {title} » avec vous dans EmailOps.\n\n\
             Ouvrez-le dans EmailOps pour le modifier ensemble : les modifications \
             voyagent par e-mail entre les personnes qui le partagent, sans serveur \
             intermédiaire. Sans EmailOps, vous pouvez lire la copie ci-dessous, mais \
             pas la modifier."
        ),
        Language::De => format!(
            "{sharer} hat „{title}“ in EmailOps mit dir geteilt.\n\n\
             Öffne es in EmailOps, um es gemeinsam zu bearbeiten: Änderungen gehen per \
             E-Mail zwischen den Beteiligten hin und her, ohne Server dazwischen. Ohne \
             EmailOps kannst du die Kopie unten lesen, aber nicht bearbeiten."
        ),
    }
}

/// The text of a message that only carries changes.
pub fn update(language: Language, title: &str) -> String {
    match language {
        Language::En => format!(
            "Changes to the shared document \"{title}\", sent by EmailOps to the people \
             editing it. EmailOps applies them automatically; you can ignore this message."
        ),
        Language::Es => format!(
            "Cambios en el documento compartido «{title}», enviados por EmailOps a quienes \
             lo editan. EmailOps los aplica automáticamente; puedes ignorar este mensaje."
        ),
        Language::Fr => format!(
            "Modifications du document partagé « {title} », envoyées par EmailOps aux \
             personnes qui le modifient. EmailOps les applique automatiquement ; vous \
             pouvez ignorer ce message."
        ),
        Language::De => format!(
            "Änderungen am geteilten Dokument „{title}“, von EmailOps an alle Bearbeitenden \
             gesendet. EmailOps übernimmt sie automatisch; du kannst diese Nachricht ignorieren."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_language_names_the_document_and_the_sharer() {
        for language in Language::ALL {
            assert!(subject(language, "Plan").starts_with("Plan ("), "{language:?}");
            let text = invitation(language, "Ana", "Plan");
            assert!(
                text.contains("Ana") && text.contains("Plan") && text.contains("EmailOps"),
                "{language:?}"
            );
            assert!(update(language, "Plan").contains("Plan"), "{language:?}");
        }
    }

    #[test]
    fn spanish_reads_spanish() {
        assert_eq!(subject(Language::Es, "Plan"), "Plan (documento compartido de EmailOps)");
        assert!(invitation(Language::Es, "Ana", "Plan").starts_with("Ana ha compartido «Plan» contigo"));
    }
}
