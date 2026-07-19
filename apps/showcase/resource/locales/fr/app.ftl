app_title = Vitrine de Day
counter_value = { $count ->
    [one] { $count } clic
   *[other] { $count } clics
}
decrement = −
increment = +
name_placeholder = Votre nom
greeting = Bonjour, { $name } !
volume_label = Volume
progress_label = Progression
busy_label = Occupé
flavor_label = Parfum
history_title = Historique
history_entry = le compteur est passé à { $value }
nav_controls = Contrôles
nav_menus = Menus et dialogues
nav_text = Texte
nav_battery = Batterie
nav_sensors = Capteurs
nav_clipboard = Presse-papiers
nav_network = Réseau
nav_media = Média
nav_pickers = Sélecteurs
nav_compose = Composition
nav_files = Fichiers
nav_tabs = Onglets
nav_stack = Pile
nav_list = Liste
nav_refresh = Actualiser
refresh_caption = Tirez le flux vers le bas — ou utilisez le bouton — pour recharger
refresh_status_idle = Inactif
refresh_status_refreshing = Actualisation…
refresh_now = Actualiser maintenant
refresh_tier_native = Tirer pour actualiser : natif
refresh_tier_emulated = Tirer pour actualiser : émulé
refresh_row = Élément { $n }
nav_webview = Vue Web
nav_lottie = Lottie
nav_about = À propos

shapes_kinds = Types
gradients_title = Dégradés
gradient_angle = Angle
shapes_transform = Transformation
shapes_angle = Angle

picker_shared_caption = Les trois styles sont liés au même signal de sélection — modifiez-en un, les autres suivent.
picker_selected = Sélection
picker_segmented = Segmenté
picker_menu = Menu
picker_inline = Aligné

# — day-piece-datetime —
nav_dates = Date et heure
dates_caption = Sélecteurs natifs de date et d'heure liés en double sens à des signaux civils — les sélecteurs d'une même section partagent le même signal.
dates_date_section = Date
dates_time_section = Heure
dates_composed_section = Composé
date_compact = Compact
date_inline = Calendrier
time_compact = Compact
time_seconds = Avec les secondes
dates_composed = Date et heure
date_bounded = En 2026
date_picked = Date choisie
time_picked = Heure choisie

compose_caption = Pièces de pure composition — sans code natif, sans fonctionnalités cargo, sur tous les backends gratuitement.
compose_rating_label = Note en étoiles
compose_rating_count = Étoiles sélectionnées :
compose_rating_placeholder = 1–5
compose_card_title = Surface réutilisable
compose_card_body = Marge + arrière-plan + coins arrondis, appliqués comme Modificateur.
compose_plain_btn = Simple
compose_styled_btn = Rempli
compose_env_value = Teinté par l'accent fourni
list_add = Ajouter 100
list_caption = { $count } lignes — seules les cellules visibles sont créées

webview_url_hint = Saisir une URL
webview_go = Aller
webview_back = Précédent
webview_forward = Suivant
webview_stop = Arrêter
webview_reload = Recharger

lottie_caption = Une animation Lottie native, fournie en JSON (lottie-ios / lottie-android)
lottie_speed = Vitesse
stack_root_body = Une vraie pile push/pop. Son chemin est un signal de l'application.
stack_push = Empiler un détail
stack_detail_title = Niveau { $depth }
stack_detail_body = Empilé sur le chemin. Le bouton retour natif réécrit le dépilement.
stack_item_title = Élément { $id }
stack_link_42 = Ouvrir item-42 avec un indice (route absolue)
stack_param_hint = Ouvert avec l'indice : {$hint}
tab_one = Aperçu
tab_two = Détails
tab_three = Réglages
tab_one_body = L'onglet aperçu. Chaque onglet conserve son propre état.
tab_two_body = L'onglet détails, sélectionné par sa clé de route.
tab_three_body = L'onglet réglages. Les liens profonds et dayscript choisissent les onglets par clé.
about_text = Une application native multiplateforme construite avec day.
modal_alert = Afficher l'alerte
modal_confirm = Confirmer
modal_delete = Supprimer…
modal_sheet = Choisir un parfum
modal_prompt = Saisir le nom
alert_title = Avis
alert_body = Vos modifications ont été enregistrées.
ok = OK
confirm_title = Quitter ?
confirm_body = Voulez-vous vraiment quitter ?
delete_title = Supprimer l'élément ?
delete_body = Cette action est irréversible.
delete = Supprimer
flavor_title = Choisissez un parfum
cancel = Annuler
vanilla = vanille
pistachio = pistache

# Files playground (docs/files.md)
files_caption = Sélecteurs de fichiers natifs. « Ouvrir » lit un fichier texte dans l'éditeur ; « Enregistrer » l'écrit.
files_placeholder = Saisissez du texte à enregistrer…
files_open = Ouvrir un fichier…
files_save = Enregistrer le fichier…
files_opened = Ouvert : { $name }

# Battery playground (docs/battery.md)
battery_refresh = Lire la batterie
battery_level = Niveau
battery_charging = En charge
battery_reading = Batterie : { $percent } · { $state }
battery_reading_none = Batterie : aucune API batterie sur cette plateforme

# Aire de jeu Capteurs (docs/sensors.md)
sensors_refresh = Lire les capteurs
sensor_accelerometer = Accéléromètre
sensor_gyroscope = Gyroscope
sensor_magnetometer = Magnétomètre
sensor_reading = x { $x } · y { $y } · z { $z } { $unit }
sensor_waiting = en attente du premier échantillon…
sensor_unavailable = indisponible sur cet appareil

# Aire de jeu Presse-papiers (docs/clipboard.md)
clipboard_caption = La part day-part-clipboard lit et écrit le presse-papiers système nativement.
clipboard_placeholder = Saisissez un texte à copier
clipboard_copy = Copier
clipboard_paste = Coller
clipboard_idle = Presse-papiers intact
clipboard_copied = Copié dans le presse-papiers système
clipboard_copy_failed = Échec de la copie (pas d'API presse-papiers ici)
clipboard_pasted = Collé depuis le presse-papiers système
clipboard_empty = Presse-papiers vide (ou illisible en arrière-plan)

# Aire de jeu Réseau (docs/network.md)
network_refresh = Lire le réseau
network_reading_online = En ligne · { $kind } · facturé : { $expensive }
network_reading_offline = Hors ligne
network_reading_none = Aucune API de connectivité sur cette plateforme

# Aire de jeu Média (docs/media.md)
media_play = Lecture
media_pause = Pause
media_load = Charger

# — Localization page (docs/localization.md) —
nav_localization = Localisation
fmt_caption = Un seul jeu de traductions — rendu conforme à ICU pour chaque locale : nombres, dates, grammaire du pluriel et ordre de tri suivent la langue.
loc_locale_section = Locale en direct
loc_live_note = La locale est un signal — changer de langue re-rend chaque chaîne instantanément. Le sens de lecture est fixé au lancement (lancez en ar pour l'interface miroir).
loc_current_label = Actuelle
loc_reset = Réinitialiser
loc_numbers_section = Nombres
loc_dates_section = Dates et heures
loc_plurals_section = Pluriels
loc_sorting_section = Tri
fmt_number_label = Groupé
fmt_fraction_label = Deux décimales
fmt_percent_label = Pourcentage
fmt_date_label = Date longue
fmt_time_label = Heure
fmt_datetime_label = Date et heure
fmt_sorted_label = Trié
fmt_number = { NUMBER($n) }
fmt_fraction = { NUMBER($n, minimumFractionDigits: 2) }
fmt_percent = { NUMBER($p, style: "percent") }
fmt_date = { DATETIME($d, dateStyle: "long") }
fmt_time = { DATETIME($t, timeStyle: "short") }
fmt_datetime = { DATETIME($dt, dateStyle: "medium", timeStyle: "short") }
plural_items = { $count ->
    [0] Rien pour l'instant
    [one] Un élément
   *[other] { $count } éléments
}

# Aire de jeu Texte (typographie)
text_caption = Les styles sémantiques correspondent aux styles natifs et à l'échelle de texte d'accessibilité.
text_styles_header = Styles
text_weights_header = Graisses
text_styling_header = Gras et italique
text_colors_header = Couleur
text_custom_header = Tailles personnalisées
text_custom_note = Font.System(pt) — mis à l'échelle par la taille de texte d'accessibilité (Dynamic Type).
text_fonts_header = Polices embarquées
text_fonts_note = Font.Custom("Famille", pt) — fichiers du dossier resource/fonts/ de l'application, embarqués par day build et résolus par nom de famille sur chaque plateforme.

# Aire de jeu Menus
menus_caption = Menus natifs — la barre de menus de l'application et les menus contextuels par élément — avec sous-menus imbriqués, raccourcis clavier et commandes d'édition standard.
menus_last = Dernière action
menus_lifecycle = Cycle de vie
menus_target = Clic droit ici (appui long sur mobile) pour un menu contextuel
menus_shortcut_hint = Les raccourcis clavier (⌘/Ctrl + touche) apparaissent dans la barre de menus et fonctionnent quand l'application est active — p. ex. Nouveau (N), Enregistrer (S), Recharger (R).

# --- day-part-haptics ---
nav_haptics = Haptique
haptics_supported_yes = Moteur haptique disponible sur cette plateforme
haptics_supported_no = Aucun moteur haptique sur cette plateforme (les boutons sont silencieux)
haptics_light = Léger
haptics_medium = Moyen
haptics_heavy = Fort
haptics_success = Succès
haptics_warning = Avertissement
haptics_error = Erreur
haptics_selection = Sélection
haptics_last = Dernier joué
haptics_none = Rien joué pour l'instant
haptics_last_played = Joué : { $style }

# --- day-part-prefs ---
nav_prefs = Préférences
prefs_caption = Conserver une chaîne entre les lancements avec day-part-prefs.
prefs_placeholder = Valeur à mémoriser
prefs_save = Enregistrer
prefs_load = Charger
prefs_clear = Effacer
prefs_idle = Saisissez une valeur, puis Enregistrer.
prefs_empty = (rien d'enregistré)
prefs_saved = Enregistré.
prefs_save_failed = Échec de l'enregistrement.
prefs_loaded = Chargé depuis le stockage.
prefs_missing = Rien d'enregistré pour l'instant.
prefs_cleared = Effacé.
prefs_value_label = Valeur enregistrée :

# --- bundled resources (§18.3) ---
nav_resources = Ressources
resources_caption = Une image chargée par nom depuis une ressource, avec accès aléatoire à des données embarquées.
resources_numbers = numbers.bin : { $len } octets, byte[100] = { $byte }
resources_greeting = greeting.txt : { $text }

# --- day-part-deviceinfo ---
nav_deviceinfo = Appareil
deviceinfo_model = Modèle : {$value}
deviceinfo_system = Système : {$name} {$version}
deviceinfo_simulator = Simulateur : {$value}
deviceinfo_yes = oui
deviceinfo_no = non
deviceinfo_refresh = Actualiser

# --- day-piece-activity ---
activity_animating = Animation
activity_on = En rotation
activity_off = Arrêté

# --- day-piece-searchfield ---
nav_search = Recherche
search_placeholder = Rechercher un fruit…
search_clear = Effacer

# --- day-piece-map ---
nav_map = Carte
map_caption = Une MKMapView native — plateformes Apple uniquement. Touchez un préréglage pour recentrer la carte en direct.
map_sf = San Francisco
map_nyc = New York

# — page tweaks (docs/tweaks.md) —
nav_tweaks = Tweaks
tweaks_intro = Les tweaks empaquetés configurent le composant natif derrière une pièce intégrée, par toolkit. Là où un tweak n'est pas couvert, il est sans effet — les pièces ci-dessous restent d'origine.
tweaks_stock = D'origine
tweaks_tweaked = Ajustée
tweaks_bezel_title = Biseau du bouton
tweaks_bezel_caption = day-tweak-button-bezel — AppKit uniquement : les constantes NSBezelStyle sur le vrai NSButton.
tweaks_selectable_title = Libellé sélectionnable
tweaks_selectable_caption = day-tweak-label-selectable — AppKit, GTK, Android : la sélection de texte native sur un libellé standard.
tweaks_selectable_text = Le texte de ce libellé peut être sélectionné et copié — essayez.
tweaks_ticks_title = Graduations du curseur
tweaks_ticks_caption = day-tweak-slider-tickmarks — AppKit, GTK, Android, Qt, WinUI, ArkUI : graduations natives, avec aimantation là où la plateforme la propose. Le curseur ajusté s'aimante ; celui d'origine glisse.
tweaks_ref_title = Vivacité du NativeRef
tweaks_ref_caption = Un NativeRef atteint le curseur ajusté après montage ; démontez-le et la référence se vide au lieu de pendre.
tweaks_ref_live = réf : vivante
tweaks_ref_cleared = réf : vidée

# — merged section pages (design overhaul) —
nav_canvas = Canevas et formes
nav_system = Appareil et capteurs
nav_services = Services système
controls_caption = Liaisons bidirectionnelles : chaque contrôle projette un signal de l'application.
controls_basics = Essentiels
controls_feedback = Retour visuel
canvas_caption = Formes, transformations, gestes et widgets composés — tous dessinés via le canevas.
canvas_gauge = Jauge canevas
shapes_interact_hint = Glissez le curseur pour pivoter, touchez le cercle pour recolorer, déplacez le carré violet.
system_caption = Les modules d'état de l'appareil : batterie, connectivité, capteurs et identité.
services_caption = Les modules « agir avec l'OS » : presse-papiers, préférences, haptique, fichiers et HTTP.
subscribe_label = S'abonner

# — data strings localized for the walkthrough locales (option lists, specimen rows) —
chocolate = chocolat
size_small = Petit
size_medium = Moyen
size_large = Grand
fruit_apple = Pomme
fruit_banana = Banane
fruit_cherry = Cerise
fruit_date = Datte
fruit_elderberry = Sureau
list_row = Ligne { $n }
text_style_large_title = Grand titre
text_style_title = Titre
text_style_title2 = Titre 2
text_style_title3 = Titre 3
text_style_headline = En-tête
text_style_subheadline = Sous-en-tête
text_style_body = Corps
text_style_callout = Encadré
text_style_footnote = Note de bas de page
text_style_caption = Légende
text_style_caption2 = Légende 2
text_weight_ultralight = Ultra-fin
text_weight_light = Fin
text_weight_regular = Normal
text_weight_medium = Moyen
text_weight_semibold = Demi-gras
text_weight_bold = Gras
text_weight_heavy = Très gras
text_weight_black = Noir
text_bold = Texte gras
text_italic = Texte italique
text_bolditalic = Gras italique
text_emphasis_label = Emphase
color_red = Rouge
color_green = Vert
color_blue = Bleu
color_orange = Orange

# Menus & dialogues (page fusionnée)
menus_appmenu_section = Menu de l’application
menus_context_section = Menu contextuel
menus_dialogs_section = Dialogues
modal_result_label = Résultat

# Page Média
media_caption = Un lecteur multimédia natif — la vue de la plateforme, transport piloté par déclencheurs.
media_player_section = Vidéo

# Sections de la page Ressources
resources_image_section = Image embarquée
resources_modes_note = Une image, trois modes — Ajuster préserve les proportions, Remplir rogne, Étirer déforme.
image_mode_fit = Ajuster
image_mode_fill = Remplir
image_mode_stretch = Étirer
resources_data_section = Données embarquées

# Page À propos
about_caption = Ce qu’est cette app, et la plateforme où elle s’exécute.
about_app_section = Cette app
about_version = Version
about_toolkit = Boîte à outils
about_battery = Batterie
history_hint = Touchez + ou − ci-dessus : chaque changement s’affiche ici.

# Page Focus (docs/focus.md)
nav_focus = Focus
focus_caption = Le focus est une liaison bidirectionnelle : les changements natifs écrivent le signal, et écrire le signal déplace le focus.
focus_group_section = Un signal, un formulaire
focus_group_caption = Trois champs liés à un même signal optionnel. Cliquez ou tabulez de l’un à l’autre et l’indicateur suit ; Entrée passe au champ suivant.
focus_name_label = Nom
focus_email_label = E-mail
focus_city_label = Ville
focus_current_label = Focus
focus_next = Focus suivant
focus_clear = Effacer le focus
focus_bool_section = Un contrôle, un booléen
focus_bool_caption = Le même champ lié à un signal booléen — les boutons l’écrivent ; entrer dans le champ ou en sortir l’écrit en retour.
focus_bool_placeholder = Le focus arrive ici
focus_focus_btn = Donner le focus
focus_blur_btn = Retirer le focus
focus_state_label = État
focus_state_on = avec focus
focus_state_off = sans focus
focus_probe_section = Au-delà des champs de texte
focus_probe_caption = Les toolkits de bureau donnent aussi le focus aux boutons, interrupteurs et curseurs ; les plateformes tactiles le réservent surtout à la saisie de texte.
focus_probe_toggle = Interrupteur
focus_probe_slider = Curseur
focus_probe_button = Bouton

# HTTP fetch demo (docs/http.md) — the status readout stays raw "<status> <body>" so the
# walkthrough asserts it byte-for-byte in every locale.
http_title = HTTP
http_caption = Le module day-part-http passe par la pile HTTP de la plateforme — ses proxys, son VPN et son TLS.
http_fetch = Récupérer depuis localhost
http_idle = Rien de récupéré pour l'instant
http_tier = Pile
http_url_placeholder = https://example.com
http_check = Vérifier
http_checking = Vérification…
