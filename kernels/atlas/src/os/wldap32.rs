//! wldap32.dll: declared so programs that link LDAP support load; exports report unimplemented if called.

use super::dll;

dll! {
    "WLDAP32.dll";
    missing ber_free;
    missing ldap_bind_s;
    missing ldap_err2string;
    missing ldap_first_attribute;
    missing ldap_first_entry;
    missing ldap_get_dn;
    missing ldap_get_values_len;
    missing ldap_init;
    missing ldap_memfree;
    missing ldap_msgfree;
    missing ldap_next_attribute;
    missing ldap_next_entry;
    missing ldap_search_s;
    missing ldap_set_option;
    missing ldap_simple_bind_s;
    missing ldap_sslinit;
    missing ldap_unbind_s;
    missing ldap_value_free_len;
}
